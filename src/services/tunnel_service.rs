//! Settings → Cloudflare Tunnel: the panel runs `cloudflared` (the Swarm
//! service `rdp-cloudflared`) with a tunnel's token, so domains are reached
//! through Cloudflare without opening ports 80 and 443. cloudflared sends
//! every hostname to Traefik's `tunnel` entrypoint (`:8081`, never published,
//! reached on the network `rdp-tunnel` only), where each domain is served
//! over plain HTTP (Cloudflare holds TLS): no redirect to HTTPS, so no loop.
//!
//! The token is sealed (`panel_settings`, APP_KEY) and given to cloudflared
//! as a Swarm secret read with `--token-file` (never an argument, an
//! environment variable or a log). The panel's administrators' only
//! (`system.manage`). Public hostnames are set in Cloudflare (Zero Trust →
//! Tunnels → the tunnel → Public Hostname), each to [`status`]'s `origin`.

use std::collections::HashMap;

use rustcms::crypto::{Context, Encrypted};
use rustcms::{AppError, AppResult, AppState, ValidationErrors};
use serde_json::{Value, json};

use crate::models::panel_setting;
use crate::services::docker_service::{Docker, DockerError, NetworkRequest, SYSTEM_LABEL};
use crate::services::traefik_service::Traefik;

/// cloudflared, pinned (it reads `--token-file`).
pub const IMAGE: &str = "cloudflare/cloudflared:2025.9.1";
pub const SERVICE: &str = "rdp-cloudflared";
pub const NETWORK: &str = "rdp-tunnel";
/// Traefik's entrypoint for the tunnel, and its port.
pub const ENTRYPOINT: &str = "tunnel";
pub const PORT: u16 = 8081;
const TOKEN_KEY: &str = "tunnel.token";
const ENABLED_KEY: &str = "tunnel.enabled";
/// Traefik's argument for the entrypoint. Its forwarded headers are never
/// trusted (Traefik is on every project's network: any container reaches
/// it): the client's address there is cloudflared's; Cloudflare's own
/// `Cf-Connecting-Ip` holds the visitor's, and `Cf-Visitor` their scheme.
const TRAEFIK_ARG: &str = "--entrypoints.tunnel.address=:8081";
/// Cloudflare's header for a visitor who came over plain HTTP (routed to
/// a redirect to HTTPS: no loop, the ones over HTTPS do not carry it).
pub const PLAIN_VISITOR: &str = r#"Header(`Cf-Visitor`, `{"scheme":"http"}`)"#;
/// Why a tunnel does not connect, as Settings says it.
const UNHEALTHY: &str =
    "Cloudflare does not accept the connection: is the tunnel's token still valid?";
/// The user cloudflared's image runs as (distroless `nonroot`).
const CLOUDFLARED_UID: &str = "65532";

static CHANGING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Held while the tunnel is turned on or off (one change at a time).
pub async fn exclusive() -> tokio::sync::MutexGuard<'static, ()> {
    CHANGING.lock().await
}

fn token_context() -> Context {
    Context::column("panel_settings", "tunnel.token")
}

/// Whether routes serve the tunnel's entrypoint too.
pub fn enabled(app: &AppState) -> bool {
    app.service::<Traefik>().is_some_and(|t| t.tunnel())
}

/// Reads whether the tunnel is on (at boot).
pub async fn load(app: &AppState) -> AppResult<()> {
    let on = panel_setting::get(app.db(), ENABLED_KEY).await?.as_deref() == Some("true");
    if let Some(traefik) = app.service::<Traefik>() {
        traefik.set_tunnel(on);
    }
    Ok(())
}

/// A tunnel token as Cloudflare gives it: base64 of a JSON object with its
/// account (`a`), tunnel (`t`) and secret (`s`).
pub fn valid_token(token: &str) -> bool {
    use base64::Engine;
    let token = token.trim();
    if !(60..=4096).contains(&token.len()) {
        return false;
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(token)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(token));
    let Ok(Ok(Value::Object(fields))) =
        decoded.map(|bytes| serde_json::from_slice::<Value>(&bytes))
    else {
        return false;
    };
    let text = |key: &str| fields.get(key).and_then(Value::as_str).unwrap_or_default();
    text("a").len() >= 16
        && text("a").chars().all(|c| c.is_ascii_hexdigit())
        && text("t").len() == 36
        && text("t").chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && text("s").len() >= 16
}

fn docker_err(err: DockerError) -> AppError {
    AppError::from(err)
}

/// What Settings shows: on or off, a token kept (never shown), cloudflared's
/// tasks.
pub async fn status(app: &AppState, docker: &Docker) -> AppResult<Value> {
    let token_set = panel_setting::get(app.db(), TOKEN_KEY).await?.is_some();
    let service = docker
        .engine()
        .inspect_service(SERVICE)
        .await
        .map_err(docker_err)?;
    let tasks = if service.is_some() {
        docker
            .engine()
            .list_tasks(SERVICE)
            .await
            .unwrap_or_default()
    } else {
        vec![]
    };
    let running = tasks.iter().any(|t| {
        t.desired_state == Some(crate::services::docker_service::TaskState::RUNNING)
            && t.status.as_ref().and_then(|s| s.state)
                == Some(crate::services::docker_service::TaskState::RUNNING)
    });
    // Swarm replaces an unhealthy task (its exit kept in the history).
    let failed = tasks.iter().any(|t| {
        t.status.as_ref().and_then(|s| s.state)
            == Some(crate::services::docker_service::TaskState::FAILED)
    });
    // A running task is no tunnel yet (a revoked token retries forever):
    // its healthcheck says whether Cloudflare took the connection.
    let (connected, log_error) = if running {
        connection(docker, &tasks).await
    } else {
        (false, None)
    };
    // Connected: what failed before is over. Else why, when known.
    let error = if connected {
        None
    } else {
        log_error.or_else(|| failed.then(|| UNHEALTHY.to_owned()))
    };
    Ok(json!({
        "enabled": enabled(app),
        "token_set": token_set,
        "service": service.is_some(),
        "running": running,
        "connected": connected,
        "error": error,
        "image": IMAGE,
        "origin": format!("http://{}:{PORT}", crate::services::deploy_rules_service::config().traefik_service),
    }))
}

/// Whether the running task's cloudflared is connected to Cloudflare (its
/// healthcheck: `cloudflared tunnel ready`), and else why not. Read within
/// a few seconds (else: not known yet).
async fn connection(
    docker: &Docker,
    tasks: &[crate::services::docker_service::SwarmTask],
) -> (bool, Option<String>) {
    use crate::services::docker_service::TaskState;
    let Some(container) = tasks
        .iter()
        .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
        .find_map(|t| {
            t.status
                .as_ref()?
                .container_status
                .as_ref()?
                .container_id
                .clone()
        })
    else {
        return (false, None);
    };
    let inspected = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        docker.engine().inspect_container(&container),
    )
    .await;
    match inspected {
        Ok(Ok(document)) => health_news(&document),
        _ => (false, None),
    }
}

/// [`connection`] from a container's inspect document.
fn health_news(document: &Value) -> (bool, Option<String>) {
    match document["State"]["Health"]["Status"].as_str() {
        Some("healthy") => (true, None),
        Some("unhealthy") => (false, Some(UNHEALTHY.to_owned())),
        _ => (false, None),
    }
}

/// Keeps `token` (sealed) when given, then runs cloudflared with the token
/// kept: Traefik gets its entrypoint and the tunnel's network, every route
/// its tunnel router.
pub async fn enable(app: &AppState, docker: &Docker, token: Option<&str>) -> AppResult<()> {
    let db = app.db();
    // A new token is kept only once cloudflared runs with it (the one kept
    // goes on working until then).
    let new = token.map(str::trim).filter(|t| !t.is_empty());
    let token = match new {
        Some(token) => {
            if !valid_token(token) {
                return Err(ValidationErrors::field(
                    "token",
                    "is not a tunnel token (Cloudflare → Zero Trust → Tunnels)",
                )
                .into());
            }
            token.to_owned()
        }
        None => {
            let Some(kept) = panel_setting::get(db, TOKEN_KEY).await? else {
                return Err(ValidationErrors::field("token", "is required").into());
            };
            let sealed: Encrypted<String> = Encrypted::from_envelope(kept);
            sealed.open(app.keyring(), &token_context()).map_err(|_| {
                ValidationErrors::field(
                    "token",
                    "cannot be read (sealed with another APP_KEY): set it again",
                )
            })?
        }
    };
    let traefik = app
        .service::<Traefik>()
        .ok_or_else(|| AppError::internal("Traefik is not bound"))?;
    if traefik.service.is_empty() {
        return Err(ValidationErrors::field(
            "token",
            "Traefik is not a Swarm service of the panel's here",
        )
        .into());
    }
    // The network cloudflared and Traefik share (nothing else).
    let labels = HashMap::from([(SYSTEM_LABEL.to_owned(), "true".to_owned())]);
    let network = docker
        .ensure_network(NetworkRequest {
            name: NETWORK.to_owned(),
            driver: "overlay".to_owned(),
            labels: labels.clone(),
            attachable: false,
            internal: false,
        })
        .await
        .map_err(docker_err)?;
    traefik
        .attach(docker, NETWORK, &network.id)
        .await
        .map_err(docker_err)?;
    traefik
        .update_service(docker.engine(), |spec| {
            let args = spec
                .task_template
                .get_or_insert_with(Default::default)
                .container_spec
                .get_or_insert_with(Default::default)
                .args
                .get_or_insert_with(Vec::new);
            let before = args.len();
            // Exactly its address: nothing else of the entrypoint (no trust).
            args.retain(|a| !a.starts_with("--entrypoints.tunnel.") || a == TRAEFIK_ARG);
            let mut changed = args.len() != before;
            if !args.iter().any(|a| a == TRAEFIK_ARG) {
                args.push(TRAEFIK_ARG.to_owned());
                changed = true;
            }
            changed
        })
        .await
        .map_err(docker_err)?;
    // The token as a secret of its own content's name (a new token: a new secret).
    use sha2::Digest;
    let digest: String = sha2::Sha256::digest(token.as_bytes())[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let secret_name = format!("rdp_tunnel_{digest}");
    let secrets = docker
        .engine()
        .list_secrets(SYSTEM_LABEL, "true")
        .await
        .map_err(docker_err)?;
    let (secret_id, made) = match secrets.iter().find(|(_, name)| *name == secret_name) {
        Some((id, _)) => (id.clone(), false),
        None => (
            docker
                .engine()
                .create_secret(&secret_name, token.as_bytes(), labels.clone())
                .await
                .map_err(docker_err)?,
            true,
        ),
    };
    let spec: crate::services::docker_service::ServiceSpec = serde_json::from_value(json!({
        "Name": SERVICE,
        "Labels": { SYSTEM_LABEL: "true" },
        "TaskTemplate": {
            "ContainerSpec": {
                "Image": IMAGE,
                "Args": ["tunnel", "--no-autoupdate", "--metrics", "127.0.0.1:2000",
                         "run", "--token-file", "/run/secrets/tunnel_token"],
                "Labels": { SYSTEM_LABEL: "true" },
                // Connected to Cloudflare (`/ready` of its metrics).
                "HealthCheck": {
                    "Test": ["CMD", "cloudflared", "tunnel", "--metrics", "127.0.0.1:2000", "ready"],
                    "Interval": 10_000_000_000u64,
                    "Timeout": 5_000_000_000u64,
                    "Retries": 3,
                    "StartPeriod": 20_000_000_000u64,
                },
                "Secrets": [{
                    "SecretID": secret_id,
                    "SecretName": secret_name,
                    "File": { "Name": "tunnel_token", "UID": CLOUDFLARED_UID, "GID": CLOUDFLARED_UID, "Mode": 0o400 },
                }],
            },
            "Networks": [{ "Target": network.id }],
            "RestartPolicy": { "Condition": "any", "Delay": 5_000_000_000u64 },
        },
        "Mode": { "Replicated": { "Replicas": 1 } },
    }))
    .map_err(AppError::internal)?;
    let ran = async {
        match docker.engine().inspect_service(SERVICE).await? {
            Some(found) => {
                let version = found.version.and_then(|v| v.index).unwrap_or(0);
                docker
                    .engine()
                    .update_service(SERVICE, version, spec, None)
                    .await
                    .map(|_| ())
            }
            None => docker.engine().create_service(spec, None).await.map(|_| ()),
        }
    }
    .await;
    if let Err(err) = ran {
        // The secret made for it, unused.
        if made {
            let _ = docker.engine().delete_secret(&secret_id).await;
        }
        return Err(docker_err(err));
    }
    if new.is_some() {
        let sealed = Encrypted::seal(app.keyring(), &token_context(), &token)?;
        panel_setting::set(db, TOKEN_KEY, sealed.envelope()).await?;
    }
    // Older tokens' secrets, unused now.
    remove_secrets(docker, Some(&secret_name)).await;
    panel_setting::set(db, ENABLED_KEY, "true").await?;
    traefik.set_tunnel(true);
    republish(app).await
}

/// Stops cloudflared (its token kept), and the routes' tunnel routers.
pub async fn disable(app: &AppState, docker: &Docker) -> AppResult<()> {
    // Off first (routes without the tunnel), then cloudflared stopped.
    panel_setting::set(app.db(), ENABLED_KEY, "false").await?;
    if let Some(traefik) = app.service::<Traefik>() {
        traefik.set_tunnel(false);
    }
    republish(app).await?;
    match docker.engine().delete_service(SERVICE).await {
        Ok(()) | Err(DockerError::NotFound) => {}
        Err(err) => return Err(err.into()),
    }
    remove_secrets(docker, None).await;
    Ok(())
}

/// The tunnel's secrets but `keep` (one in use is refused: kept until the
/// next change).
async fn remove_secrets(docker: &Docker, keep: Option<&str>) {
    let Ok(secrets) = docker.engine().list_secrets(SYSTEM_LABEL, "true").await else {
        return;
    };
    for (id, name) in secrets {
        if name.starts_with("rdp_tunnel_") && Some(name.as_str()) != keep {
            let _ = docker.engine().delete_secret(&id).await;
        }
    }
}

/// At boot: cloudflared as the setting says (run again when on and missing,
/// stopped when off), in the background.
pub fn reconcile(app: &AppState) {
    let app = app.clone();
    tokio::spawn(async move {
        let Some(docker) = app.service::<Docker>() else {
            return;
        };
        let _changing = exclusive().await;
        let found = docker
            .engine()
            .inspect_service(SERVICE)
            .await
            .ok()
            .flatten();
        let present = found.is_some();
        // Made by an older panel (no healthcheck): made again as now.
        let older = found.is_some_and(|s| {
            s.spec
                .and_then(|spec| spec.task_template)
                .and_then(|t| t.container_spec)
                .and_then(|c| c.health_check)
                .is_none()
        });
        let result = match (enabled(&app), present) {
            (true, false) => enable(&app, &docker, None).await,
            (true, true) if older => enable(&app, &docker, None).await,
            (false, true) => disable(&app, &docker).await,
            (false, false) => {
                // Left by a change cut short.
                remove_secrets(&docker, None).await;
                Ok(())
            }
            (true, true) => Ok(()),
        };
        if let Err(err) = result {
            tracing::warn!(%err, "the tunnel not reconciled");
        }
    });
}

/// Every service's routes written again (with or without their tunnel
/// routers), and the panel's own.
async fn republish(app: &AppState) -> AppResult<()> {
    let Some(traefik) = app.service::<Traefik>() else {
        return Ok(());
    };
    let options = crate::services::deployment_service::deploy_options(0);
    crate::services::traefik_service::sync_all(app.db(), &traefik, &options).await?;
    write_panel_route(app)
        .map_err(|err| AppError::internal(format!("the panel's tunnel route: {err}")))
}

/// At boot, after [`load`]: the panel's own tunnel route as the setting says.
pub fn sync_panel_route(app: &AppState) {
    if let Err(err) = write_panel_route(app) {
        tracing::warn!(%err, "the panel's tunnel route not written");
    }
}

/// The panel's own domain through the tunnel (the installer's route serves
/// it on `web`/`websecure`): a router on the tunnel's entrypoint to the
/// installer's `rdockpanel@file` service; removed when off.
fn write_panel_route(app: &AppState) -> std::io::Result<()> {
    let cfg = crate::services::deploy_rules_service::config();
    let Some(traefik) = app.service::<Traefik>() else {
        return Ok(());
    };
    let path = traefik.dir.join("rdp-tunnel-panel.yml");
    let host = cfg.panel_host.clone();
    if !traefik.tunnel()
        || !cfg.installed
        || host.is_empty()
        || host.parse::<std::net::IpAddr>().is_ok()
    {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    // Over HTTPS (Cloudflare's), and a visitor over plain HTTP redirected:
    // passwords never cross plain HTTP.
    let plain = PLAIN_VISITOR.replace('"', "\\\"");
    let yaml = format!(
        "http:\n  routers:\n    rdockpanel-tunnel:\n      rule: \"Host(`{host}`)\"\n      entryPoints: [\"{ENTRYPOINT}\"]\n      service: rdockpanel@file\n      priority: 1000000\n    rdockpanel-tunnel-http:\n      rule: \"Host(`{host}`) && {plain}\"\n      entryPoints: [\"{ENTRYPOINT}\"]\n      service: rdockpanel@file\n      middlewares: [\"rdockpanel-https@file\"]\n      priority: 1000001\n"
    );
    std::fs::write(path, yaml)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_from_health() {
        let state = |status: &str| json!({"State": {"Health": {"Status": status}}});
        assert_eq!(health_news(&state("healthy")), (true, None));
        let (connected, error) = health_news(&state("unhealthy"));
        assert!(!connected && error.is_some());
        assert_eq!(health_news(&state("starting")), (false, None));
        assert_eq!(health_news(&json!({})), (false, None));
    }

    #[test]
    fn tunnel_tokens() {
        assert!(valid_token(
            "eyJhIjoiMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYiLCJ0IjoiMDEyMzQ1NjctODlhYi1jZGVmLTAxMjMtNDU2Nzg5YWJjZGVmIiwicyI6ImMyVmpjbVYwTFhObFkzSmxkQzF6WldOeVpYUT0ifQ=="
        ));
        assert!(!valid_token(&"A".repeat(60)));
        assert!(!valid_token("not a token"));
        assert!(!valid_token(""));
    }
}
