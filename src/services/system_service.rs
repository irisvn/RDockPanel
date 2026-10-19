//! Settings → Panel's controls, like EasyPanel's: the panel (its logs, a
//! restart, whether a newer image is out), the server (its public address
//! found again, a reboot) and Docker (what may be pruned). The panel's
//! administrators' (`system.manage`).
//!
//! The panel never changes its own image: an update is the installer's
//! command, run on the server. Its restart and the server's reboot are
//! logged and audited before they start (they stop the panel).

use std::net::IpAddr;
use std::sync::RwLock;
use std::time::Duration;

use rustcms::{AppError, AppResult, ValidationErrors};
use serde_json::{Value, json};

use crate::services::action_service::{self, Initiator};
use crate::services::deploy_rules_service::config;
use crate::services::docker_service::{Docker, DockerResult};

/// The panel's Swarm service, as the installer names it.
pub const SELF_SERVICE: &str = "rdockpanel";
/// The action kinds that stop the panel (closed at its next boot).
pub const DISRUPTIVE: [&str; 2] = ["panel.restart", "server.reboot"];
/// The server's public address found by Refresh IP (`panel_settings`).
const IP_KEY: &str = "public_ip";
/// Where the public address is asked (each answers it as plain text).
const IP_LOOKUPS: [&str; 3] = [
    "https://api.ipify.org",
    "https://ipv4.icanhazip.com",
    "https://ifconfig.me/ip",
];
const LOOKUP_TIME: Duration = Duration::from_secs(5);
/// What the reboot runs in the host's namespaces.
const REBOOT: [&str; 12] = [
    "nsenter",
    "-t",
    "1",
    "-m",
    "-u",
    "-i",
    "-n",
    "-p",
    "--",
    "sh",
    "-c",
    "sleep 3; systemctl reboot || reboot",
];

static PUBLIC_IP: RwLock<Option<String>> = RwLock::new(None);

/// The server's public address: as Refresh IP last found it, else the
/// installer's (`[panel] public_ip`); empty: unknown.
pub fn public_ip() -> String {
    PUBLIC_IP
        .read()
        .ok()
        .and_then(|ip| ip.clone())
        .unwrap_or_else(|| config().public_ip.clone())
}

/// Reads the address Refresh IP found (at boot).
pub async fn load_public_ip<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<()> {
    let found = crate::models::panel_setting::get(db, IP_KEY)
        .await?
        .filter(|ip| parse_ip(ip).is_some());
    if let Ok(mut ip) = PUBLIC_IP.write() {
        *ip = found;
    }
    Ok(())
}

/// The server's hostname: Docker's name for its host (a container's own
/// `/proc` names the container, even read through `HOST_PROC`), else this
/// machine's.
pub async fn hostname(app: &rustcms::AppState) -> String {
    if let Some(docker) = app.service::<Docker>()
        && let Ok(info) = docker.engine().info().await
        && !info.name.is_empty()
    {
        return info.name;
    }
    app.service::<crate::services::system_stats_service::SystemStats>()
        .map(|s| s.info().hostname)
        .unwrap_or_default()
}

/// The panel's image as this process runs it (`name:tag@sha256:…`, and the
/// local image's id): its own container's (`HOSTNAME`, Docker's short id),
/// else its service's (what Swarm wants: a rollout may not have reached
/// this task). None: not the installer's service.
async fn own_image(docker: &Docker) -> DockerResult<Option<(String, Option<String>)>> {
    let Some(service) = docker.engine().inspect_service(SELF_SERVICE).await? else {
        return Ok(None);
    };
    if let Ok(me) = std::env::var("HOSTNAME")
        && let Ok(doc) = docker.engine().inspect_container(&me).await
        && doc["Config"]["Labels"]["com.docker.swarm.service.name"] == SELF_SERVICE
        && let Some(reference) = doc["Config"]["Image"].as_str()
    {
        let id = doc["Image"].as_str().map(str::to_owned);
        // Updated by its tag (no digest in it): the digest the registry
        // served it as, from the image's own record.
        let mut reference = reference.to_owned();
        if !reference.contains('@')
            && let Some(id) = &id
            && let Ok(image) = docker.engine().inspect_image(id).await
        {
            let repo = repository(&reference);
            if let Some(digest) = image.repo_digests.iter().find_map(|d| {
                d.split_once('@')
                    .filter(|(r, _)| *r == repo)
                    .map(|(_, d)| d.to_owned())
            }) {
                reference = format!("{reference}@{digest}");
            }
        }
        return Ok(Some((reference, id)));
    }
    Ok(service
        .spec
        .and_then(|s| s.task_template)
        .and_then(|t| t.container_spec)
        .and_then(|c| c.image)
        .map(|image| (image, None)))
}

/// `name` of `name:tag` (a registry's port kept: `host:5000/name:tag`).
fn repository(reference: &str) -> &str {
    let reference = reference.split('@').next().unwrap_or(reference);
    match reference.rsplit_once(':') {
        Some((name, tag)) if !tag.contains('/') => name,
        _ => reference,
    }
}

/// What the controls show: the panel (version, image, `installed` by
/// install.sh — `[panel] installed`), the server (hostname, address) and
/// what each control can do here.
pub async fn status(app: &rustcms::AppState, docker: Option<&Docker>, installed: bool) -> Value {
    let image = match docker {
        Some(docker) if installed => own_image(docker).await.ok().flatten().map(|(r, _)| r),
        _ => None,
    };
    let ip = public_ip();
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "installed": installed,
        "image": image,
        "hostname": hostname(app).await,
        "public_ip": if ip.is_empty() { Value::Null } else { json!(ip) },
        // Installed, the panel is a Swarm service: restarted by Swarm, its
        // logs Docker's; else its own process (from source).
        "service": installed && image.is_some(),
        "can_reboot": installed && image.is_some(),
    })
}

/// Restarts the panel's service (a forced update): logged and audited
/// first, since the panel stops meanwhile.
pub async fn restart(
    app: &rustcms::AppState,
    docker: &Docker,
    by: &Initiator,
    installed: bool,
) -> AppResult<()> {
    if !installed || own_image(docker).await?.is_none() {
        return Err(ValidationErrors::field(
            "panel",
            "the panel runs from source here: restart its process",
        )
        .into());
    }
    let row = action_service::announce(app.db(), by, "panel.restart", SELF_SERVICE).await?;
    let outcome = force_update(docker, SELF_SERVICE).await;
    let error = outcome.as_ref().err().map(ToString::to_string);
    // The panel may already be stopping: closed at its next boot otherwise.
    if error.is_some() {
        let _ = action_service::finish(app.db(), row, error, None).await;
    }
    outcome.map_err(Into::into)
}

/// Swarm replaces the service's tasks (same spec, a new force counter).
async fn force_update(docker: &Docker, name: &str) -> DockerResult<()> {
    let engine = docker.engine();
    let service = engine
        .inspect_service(name)
        .await?
        .ok_or(crate::services::docker_service::DockerError::NotFound)?;
    let version = service.version.and_then(|v| v.index).unwrap_or(0);
    let mut spec = service.spec.unwrap_or_default();
    let template = spec.task_template.get_or_insert_with(Default::default);
    template.force_update = Some(template.force_update.unwrap_or(0) + 1);
    engine.update_service(name, version, spec, None).await?;
    Ok(())
}

/// Reboots the server: a privileged container of the panel's own image in
/// the host's namespaces, once `confirm` is its hostname. Installed only
/// (from source, the "server" may be the developer's machine or VM).
pub async fn reboot(
    app: &rustcms::AppState,
    docker: &Docker,
    by: &Initiator,
    confirm: &str,
    installed: bool,
) -> AppResult<()> {
    // The local image this process runs (its id when known): no pull.
    let image = if installed {
        own_image(docker)
            .await?
            .map(|(reference, id)| id.unwrap_or(reference))
    } else {
        None
    };
    let Some(image) = image else {
        return Err(ValidationErrors::field(
            "server",
            "only an installed panel reboots its server",
        )
        .into());
    };
    let host = hostname(app).await;
    if host.is_empty() || confirm.trim() != host {
        return Err(ValidationErrors::field("confirm", "type the server's hostname").into());
    }
    let row = action_service::announce(app.db(), by, "server.reboot", &host).await?;
    let command = REBOOT.iter().map(|s| (*s).to_owned()).collect();
    if let Err(err) = docker.engine().run_on_host(&image, command).await {
        let _ = action_service::finish(app.db(), row, Some(err.to_string()), None).await;
        return Err(err.into());
    }
    Ok(())
}

/// Whether the registry serves a newer image than the panel runs, and the
/// installer command that updates it (the panel never replaces itself).
pub async fn check_update(docker: &Docker, installed: bool) -> AppResult<Value> {
    if !installed {
        return Ok(json!({ "installed": false }));
    }
    let Some((image, _)) = own_image(docker).await? else {
        return Ok(json!({ "installed": true, "image": Value::Null }));
    };
    let (reference, running) = match image.split_once('@') {
        Some((reference, digest)) => (reference.to_owned(), Some(digest.to_owned())),
        None => (image.clone(), None),
    };
    // Swarm pins the digest the registry served (a multi-platform image's
    // index), as the registry serves it now: the same kind compared.
    // Unknown (offline, a registry that needs a login): said so.
    let latest = docker
        .engine()
        .registry_digest(&reference)
        .await
        .ok()
        .flatten();
    let update = match (&running, &latest) {
        (Some(running), Some(latest)) => json!(running != latest),
        _ => Value::Null,
    };
    Ok(json!({
        "installed": true,
        "image": reference,
        "running": running,
        "latest": latest,
        "update": update,
        "command": format!(
            "curl -fsSL {} | sudo sh",
            crate::services::panel_domain_service::INSTALLER
        ),
    }))
}

/// Asks the public address again ([`IP_LOOKUPS`], the first valid answer),
/// keeps it, and says what changed.
pub async fn refresh_ip<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<Value> {
    let previous = public_ip();
    let client = reqwest::Client::builder()
        .timeout(LOOKUP_TIME)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(AppError::internal)?;
    let mut found = None;
    for url in IP_LOOKUPS {
        let Ok(response) = client.get(url).send().await else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        if let Ok(body) = response.text().await
            && let Some(ip) = parse_ip(&body)
        {
            found = Some(ip);
            break;
        }
    }
    let Some(ip) = found else {
        return Err(ValidationErrors::field(
            "public_ip",
            "no address lookup answered (is the server offline?)",
        )
        .into());
    };
    let ip = ip.to_string();
    // Kept and published together: the database and this process agree.
    static SAVING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _saving = SAVING.lock().await;
    crate::models::panel_setting::set(db, IP_KEY, &ip).await?;
    if let Ok(mut kept) = PUBLIC_IP.write() {
        *kept = Some(ip.clone());
    }
    Ok(json!({ "public_ip": ip, "previous": previous, "changed": previous != ip }))
}

/// A lookup's answer: one public address (no private, loopback, shared,
/// multicast, documentation or other special-use one), else none.
fn parse_ip(body: &str) -> Option<IpAddr> {
    let text = body.trim();
    if text.len() > 45 {
        return None;
    }
    let ip: IpAddr = text.parse().ok()?;
    public(ip).then_some(ip)
}

fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || a == 0
                || a >= 240
                || a == 100 && (64..128).contains(&b)
                || a == 198 && (18..20).contains(&b)
                || a == 192 && b == 0 && c == 0)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || s[0] == 0x2001 && s[1] == 0x0db8
                // Global unicast only (2000::/3).
                || s[0] & 0xe000 != 0x2000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories() {
        assert_eq!(
            repository("ghcr.io/irisvn/rdockpanel:main"),
            "ghcr.io/irisvn/rdockpanel"
        );
        assert_eq!(repository("host:5000/app:v1@sha256:x"), "host:5000/app");
        assert_eq!(repository("host:5000/app"), "host:5000/app");
        assert_eq!(repository("nginx"), "nginx");
    }

    #[test]
    fn lookup_answers() {
        assert_eq!(
            parse_ip("203.0.114.7\n"),
            Some("203.0.114.7".parse().unwrap())
        );
        assert_eq!(
            parse_ip(" 2a01:4f8::1 "),
            Some("2a01:4f8::1".parse().unwrap())
        );
        for bad in [
            "",
            "10.0.0.1",
            "192.168.1.2",
            "127.0.0.1",
            "100.64.0.1",
            "203.0.113.9",
            "224.0.0.1",
            "240.0.0.1",
            "198.18.0.1",
            "fd00::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "<html>",
            "1.2.3.4 5.6.7.8",
        ] {
            assert_eq!(parse_ip(bad), None, "{bad}");
        }
    }
}
