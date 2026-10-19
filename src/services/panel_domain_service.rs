//! Settings → Panel: the panel's domain, its Let's Encrypt email and the
//! apps' domain, as the installer set them, and a check of new ones that
//! ends with the installer command applying them. The panel never changes
//! them itself (its own service, Traefik's resolver and its route would
//! change at once, with no way back if it did not come up): the command is
//! run on the server, as the installer always is.

use std::time::Duration;

use serde_json::{Value, json};

use crate::services::deploy_rules_service::{config, valid_domain_host};

/// How long one name may take to resolve.
const LOOKUP_TIME: Duration = Duration::from_secs(5);
/// Where the installer is fetched from.
const INSTALLER: &str =
    "https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh";

/// What the installer set: the panel's host, its Let's Encrypt email (from
/// Traefik's arguments), the apps' domain, the server's address.
pub async fn current(
    docker: Option<&crate::services::docker_service::Docker>,
    traefik: &crate::services::traefik_service::Traefik,
) -> Value {
    let cfg = config();
    let mut email = None;
    if let Some(docker) = docker
        && !traefik.service.is_empty()
        && let Ok(Some(service)) = docker.engine().inspect_service(&traefik.service).await
    {
        email = service
            .spec
            .and_then(|s| s.task_template)
            .and_then(|t| t.container_spec)
            .and_then(|c| c.args)
            .unwrap_or_default()
            .iter()
            .find_map(|a| {
                a.strip_prefix(&format!(
                    "--certificatesresolvers.{}.acme.email=",
                    crate::models::cert_resolver::INSTALLER_RESOLVER
                ))
                .map(str::to_owned)
            });
    }
    let host = cfg.panel_host.clone();
    let ip = host.parse::<std::net::IpAddr>().is_ok();
    json!({
        "domain": if host.is_empty() || ip || host == "localhost" { Value::Null } else { json!(host) },
        "email": email,
        "apps_domain": if cfg.service_domain.is_empty() || cfg.service_domain == "localhost" { Value::Null } else { json!(cfg.service_domain) },
        "public_ip": if cfg.public_ip.is_empty() { Value::Null } else { json!(cfg.public_ip) },
        "https": cfg.https_port == 443 && !cfg.cert_resolver.is_empty(),
    })
}

/// What is asked: a panel domain (none: back to http://<ip>:3000), the Let's
/// Encrypt email, the apps' domain (none: no generated domains).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Wanted {
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub apps_domain: String,
}

/// A name's addresses (none: it does not resolve), in a few seconds.
async fn addresses(host: &str) -> Vec<String> {
    match tokio::time::timeout(LOOKUP_TIME, tokio::net::lookup_host((host, 443))).await {
        Ok(Ok(found)) => {
            let mut list: Vec<String> = found.map(|a| a.ip().to_string()).collect();
            list.sort();
            list.dedup();
            list
        }
        _ => vec![],
    }
}

/// What a name resolves to, said against the server's address.
async fn dns(name: &str, shown: &str, public_ip: &str) -> (Value, Option<String>) {
    let found = addresses(name).await;
    let warning = if found.is_empty() {
        Some(format!(
            "{shown} does not resolve yet: add its DNS record (A {})",
            if public_ip.is_empty() {
                "<this server>"
            } else {
                public_ip
            }
        ))
    } else if !public_ip.is_empty() && !found.iter().any(|a| a == public_ip) {
        Some(format!(
            "{shown} resolves to {} and not to this server ({public_ip}): fine behind a proxy (Cloudflare) or NAT, else fix the record first",
            found.join(", ")
        ))
    } else {
        None
    };
    (
        json!({ "name": shown, "addresses": found, "ok": warning.is_none() }),
        warning,
    )
}

/// Checks `wanted`: errors (by field) that stop it, warnings to read first
/// (DNS, a taken name), and the installer command applying it.
pub async fn check<C: sea_orm::ConnectionTrait>(
    db: &C,
    wanted: &Wanted,
) -> rustcms::AppResult<Value> {
    use sea_orm::EntityTrait;
    let domain = wanted.domain.trim().to_ascii_lowercase();
    let email = wanted.email.trim().to_owned();
    let apps = wanted.apps_domain.trim().to_ascii_lowercase();
    let mut errors = serde_json::Map::new();
    let mut warnings: Vec<String> = vec![];
    if !domain.is_empty() && (domain.starts_with("*.") || !valid_domain_host(&domain)) {
        errors.insert(
            "domain".into(),
            json!("is not a host name like panel.example.com"),
        );
    }
    if !apps.is_empty() && (apps.starts_with("*.") || !valid_domain_host(&apps)) {
        errors.insert(
            "apps_domain".into(),
            json!("is not a domain like apps.example.com"),
        );
    }
    if !email.is_empty() && !valid_email(&email) {
        errors.insert("email".into(), json!("is not an email address"));
    }
    if !domain.is_empty() && email.is_empty() {
        errors.insert(
            "email".into(),
            json!("is needed for the panel's HTTPS certificate"),
        );
    }
    if !domain.is_empty() && domain == apps {
        errors.insert("apps_domain".into(), json!("is the panel's own domain"));
    }
    // A service's domain (or a wildcard of one) is not the panel's.
    if !domain.is_empty() {
        // Its domains now, and those its deployed routes still serve.
        let mut hosts: Vec<String> = crate::models::service_domain::Entity::find()
            .all(db)
            .await?
            .into_iter()
            .map(|row| row.host)
            .collect();
        for row in crate::models::service::Entity::find().all(db).await? {
            if let Some(snapshot) =
                crate::services::traefik_service::deployed_snapshot(db, &row).await?
            {
                hosts.extend(snapshot.domains.into_iter().map(|d| d.host));
            }
        }
        if let Some(host) = hosts
            .iter()
            .find(|h| crate::services::certificate_service::covers(&[h.as_str()], &domain))
        {
            errors.insert(
                "domain".into(),
                json!(format!(
                    "is a service's domain ({host}): remove it there (and deploy) first"
                )),
            );
        }
        if !apps.is_empty() && domain.ends_with(&format!(".{apps}")) {
            warnings.push(format!(
                "{domain} is under the apps' domain: a generated domain can never be it, but keep it distinct if you can"
            ));
        }
    }
    let public_ip = config().public_ip.clone();
    let mut checks = vec![];
    if errors.is_empty() {
        if !domain.is_empty() {
            let (check, warning) = dns(&domain, &domain, &public_ip).await;
            checks.push(check);
            warnings.extend(warning);
        }
        if !apps.is_empty() {
            // Any name under it: its wildcard record.
            let probe = format!("rdp-check-{:08x}.{apps}", rand_suffix());
            let (check, warning) = dns(&probe, &format!("*.{apps}"), &public_ip).await;
            checks.push(check);
            warnings.extend(warning);
        }
    }
    let command = errors.is_empty().then(|| command(&domain, &email, &apps));
    Ok(json!({
        "errors": errors,
        "warnings": warnings,
        "dns": checks,
        "command": command,
    }))
}

/// The installer command applying them: the values were checked (host names
/// and an email: nothing a shell would read differently).
pub fn command(domain: &str, email: &str, apps: &str) -> String {
    let mut args = vec![];
    if domain.is_empty() {
        args.push("--no-domain".to_owned());
    } else {
        args.push(format!("--domain {domain}"));
    }
    // An empty one is given too: the installer keeps a value it is not given.
    if email.is_empty() {
        args.push("--email ''".to_owned());
    } else {
        args.push(format!("--email {email}"));
    }
    if apps.is_empty() {
        args.push("--apps-domain ''".to_owned());
    } else {
        args.push(format!("--apps-domain {apps}"));
    }
    format!(
        "curl -fsSL {INSTALLER} \\\n  | sudo sh -s -- {}",
        args.join(" ")
    )
}

/// The installer's own email check (the same values pass both).
fn valid_email(email: &str) -> bool {
    let Some((user, host)) = email.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && user
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c))
        && host.rsplit_once('.').is_some_and(|(name, tld)| {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ".-".contains(c))
                && tld.len() >= 2
                && tld.chars().all(|c| c.is_ascii_lowercase())
        })
}

fn rand_suffix() -> u32 {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).expect("the system's random source");
    u32::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands() {
        assert_eq!(
            command("panel.example.com", "ops@example.com", "apps.example.com"),
            format!(
                "curl -fsSL {INSTALLER} \\\n  | sudo sh -s -- --domain panel.example.com --email ops@example.com --apps-domain apps.example.com"
            )
        );
        assert!(command("", "", "").ends_with("--no-domain --email '' --apps-domain ''"));
    }

    #[test]
    fn emails_as_the_installer() {
        assert!(valid_email("ops@example.com"));
        assert!(valid_email("a.b+c@mail.example.io"));
        for bad in [
            "",
            "ops",
            "ops@example",
            "ops@Example.com",
            "o p@example.com",
            "ops@example.c",
            "a;b@example.com",
        ] {
            assert!(!valid_email(bad), "{bad}");
        }
    }
}
