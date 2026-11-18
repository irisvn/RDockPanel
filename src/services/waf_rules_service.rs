//! The WAF's rules and blocklist, as its page runs them:
//! - **rules**: the bundle in the panel's image (pinned) or CrowdSec's
//!   latest. [`check`] asks CrowdSec's hub which versions are out (the
//!   index only) and stores the answer; [`update`] upgrades the AppSec
//!   collections in the running CrowdSec, restarts it and waits for the
//!   canary, falling back to the bundle when it fails; going back to the
//!   bundle is a reconcile ([`crate::services::waf_docker_service`]).
//! - **blocklist**: the addresses CrowdSec's community reports (with
//!   sharing on) and those an administrator blocks, with an expiry
//!   ([`block`], [`unblock`]); never Cloudflare's, never a private network.

use chrono::{DateTime, Utc};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::models::panel_setting;
use crate::services::docker_service::Docker;
use crate::services::waf_docker_service::{self as waf, cscli};
use crate::services::waf_service::{RulesSource, Settings};

/// The AppSec collections the WAF runs, and their names on the page.
pub const COLLECTIONS: [(&str, &str); 3] = [
    ("crowdsecurity/appsec-virtual-patching", "Virtual patching"),
    ("crowdsecurity/appsec-generic-rules", "Generic rules"),
    ("crowdsecurity/appsec-crs", "OWASP Core Rule Set"),
];
const CHECK_KEY: &str = "waf.rules_check";
/// The bundle's index, in the panel's image.
const BUNDLE_INDEX: &str = "/app/waf/hub/.index.json";
pub const UPDATE_ACTION: &str = "waf.rules_update";

/// A collection's versions: in use, bundled, the latest out.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Versions {
    pub name: String,
    pub label: String,
    pub local: Option<String>,
    pub bundled: Option<String>,
    pub latest: Option<String>,
}

/// The last check.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RulesCheck {
    pub checked_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub collections: Vec<Versions>,
}

impl RulesCheck {
    /// Collections with a newer version out.
    pub fn outdated(&self) -> usize {
        self.collections
            .iter()
            .filter(|c| c.latest.is_some() && c.latest != c.local)
            .count()
    }
}

pub async fn last_check<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<RulesCheck> {
    Ok(panel_setting::get(db, CHECK_KEY)
        .await?
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default())
}

/// The bundle's versions (`None` outside the panel's image).
pub fn bundled() -> std::collections::HashMap<String, String> {
    std::fs::read_to_string(BUNDLE_INDEX)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map(|index| {
            COLLECTIONS
                .iter()
                .filter_map(|(name, _)| {
                    index["collections"][name]["version"]
                        .as_str()
                        .map(|v| ((*name).to_owned(), v.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Asks CrowdSec's hub (its index only: nothing installed) which versions
/// are out; stored either way.
pub async fn check(app: &AppState, docker: &Docker) -> AppResult<RulesCheck> {
    let mut check = RulesCheck {
        checked_at: Some(Utc::now()),
        ..RulesCheck::default()
    };
    match versions(docker).await {
        Ok(collections) => check.collections = collections,
        Err(err) => check.error = Some(err.to_string()),
    }
    let text = serde_json::to_string(&check).map_err(AppError::internal)?;
    panel_setting::set(app.db(), CHECK_KEY, &text).await?;
    Ok(check)
}

/// The daily check (while the WAF is on): CrowdSec's hub asked, the result
/// on the Rules tab; newer rules than last time are told to the channels
/// that hear Traefik's events. Nothing is installed: updating is an
/// administrator's choice (WAF → Rules).
pub async fn scheduled_check(app: &AppState) -> AppResult<Option<String>> {
    if Settings::load(app.db()).await?.mode == crate::services::waf_service::Mode::Off {
        return Ok(None);
    }
    let Some(docker) = app.service::<Docker>() else {
        return Ok(None);
    };
    let Ok(_rules) = RULES.try_lock() else {
        return Ok(None);
    };
    let before = last_check(app.db()).await?.outdated();
    let check = check(app, &docker).await?;
    let newer = check.outdated();
    if newer > before {
        crate::services::notify_service::notify(
            app,
            crate::services::notify_service::Event::WafRules { newer },
        );
    }
    Ok(check.error.map(|e| format!("WAF rules not checked: {e}")))
}

async fn versions(docker: &Docker) -> AppResult<Vec<Versions>> {
    if cscli(docker, &["hub", "update"]).await?.is_none() {
        return Err(AppError::BadRequest(
            "CrowdSec is not running: turn the WAF on".into(),
        ));
    }
    let bundled = bundled();
    let mut out = vec![];
    for (name, label) in COLLECTIONS {
        let text = cscli(docker, &["collections", "inspect", name, "-o", "json"])
            .await?
            .unwrap_or_default();
        let item: Value = serde_json::from_str(text.trim()).unwrap_or(Value::Null);
        out.push(Versions {
            name: name.to_owned(),
            label: label.to_owned(),
            local: item["local_version"].as_str().map(str::to_owned),
            latest: item["version"].as_str().map(str::to_owned),
            bundled: bundled.get(name).cloned(),
        });
    }
    Ok(out)
}

/// One rules task at a time (check, update, back to the bundle): an
/// update's hub is never checked, replaced or overwritten midway. Taken by
/// the controller and held by the task it starts.
pub static RULES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Refused while another rules task runs.
pub fn claim() -> AppResult<tokio::sync::MutexGuard<'static, ()>> {
    RULES.try_lock().map_err(|_| AppError::Conflict {
        code: "waf_rules_busy",
        reason: "busy",
        message: "The rules are being checked or updated: try again in a minute",
    })
}

/// Upgrades the AppSec collections in the running CrowdSec (and their
/// data), restarts it and waits for the canary; failing, the bundle is put
/// back (audited as the system's change). The settings say `latest` from
/// the request on (stored with its audit entry), so that no reconcile copies
/// the bundle over the upgrade. Logged by `row`; `_rules` held throughout.
pub async fn update(
    app: &AppState,
    docker: &Docker,
    row: crate::models::action::Model,
    switching: bool,
    _rules: tokio::sync::MutexGuard<'static, ()>,
) -> AppResult<()> {
    let mut log = String::from("Updating CrowdSec's hub index…\n");
    let row = crate::services::action_service::note(app.db(), row, &log).await?;
    let outcome = upgrade(app, docker, &mut log, switching).await;
    let error = outcome.as_ref().err().map(ToString::to_string);
    if let Some(err) = &error {
        log.push_str(&format!("Failed: {err}\nBack to the bundled rules.\n"));
        let txn = rustcms::db::begin_write(app.db()).await?;
        let mut settings = Settings::load_for_change(&txn).await?;
        settings.rules = RulesSource::Bundled;
        settings.store(&txn).await?;
        rustcms::audit::record(
            &txn,
            &rustcms::audit::AuditContext::new(
                rustcms::audit::AuditActor::system(None),
                rustcms::audit::RequestMeta::default(),
            ),
            rustcms::audit::AuditEntry::new("waf.rules_bundled", "panel_settings")
                .key(crate::services::waf_service::SETTINGS_KEY)
                .meta(json!({ "reason": err })),
        )
        .await?;
        txn.commit().await?;
        // Still holding the rules: the bundle copied back before anything
        // else touches them.
        if let Err(err) = waf::reconcile(app).await {
            log.push_str(&format!("The bundled rules not started: {err}\n"));
        }
    } else {
        log.push_str("Done: the WAF checks requests with the latest rules.\n");
    }
    crate::services::action_service::finish(app.db(), row, error, Some(&log)).await?;
    let _ = check(app, docker).await;
    outcome
}

async fn upgrade(
    app: &AppState,
    docker: &Docker,
    log: &mut String,
    switching: bool,
) -> AppResult<()> {
    cscli(docker, &["hub", "update"])
        .await?
        .ok_or_else(|| AppError::BadRequest("CrowdSec is not running".into()))?;
    log.push_str("Upgrading the rules…\n");
    let out = cscli(docker, &["hub", "upgrade"])
        .await?
        .unwrap_or_default();
    for line in out.lines().filter(|l| !l.trim().is_empty()).take(40) {
        log.push_str(&format!("  {}\n", line.trim()));
    }
    log.push_str("Restarting CrowdSec with them…\n");
    if switching {
        // From the bundle: the reconcile restarts CrowdSec (and waits for it).
        let status = waf::reconcile(app).await?;
        return match status.state.as_str() {
            "on" => Ok(()),
            _ => Err(AppError::internal(status.detail.unwrap_or_else(|| {
                "the WAF did not check requests again".to_owned()
            }))),
        };
    }
    // As a reconcile: the minute's check waits (CrowdSec restarting is not
    // the WAF failing).
    let _one = waf::RECONCILING.lock().await;
    waf::restart_crowdsec(docker).await?;
    // CrowdSec loads its rules again: the canary says when it checks.
    for _ in 0..36 {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        let now = waf::probe(app, None).await?;
        if now.state == "on" {
            return Ok(());
        }
    }
    Err(AppError::internal(
        "the WAF did not check requests again within three minutes".to_owned(),
    ))
}

/// An address an administrator blocks: one IP, public, not Cloudflare's.
pub fn blockable(ip: &str) -> Result<String, String> {
    let address: std::net::IpAddr = ip
        .trim()
        .parse()
        .map_err(|_| format!("{ip} is not an IP address"))?;
    // `::ffff:203.0.113.7` is blocked as the IPv4 address CrowdSec sees.
    let address = match address {
        std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(address, std::net::IpAddr::V4),
        v4 => v4,
    };
    let cidr = crate::services::firewall_rules::Cidr::parse(&address.to_string())?;
    if !crate::services::cloudflare_ranges_service::public(address) {
        return Err(format!("{ip} is a private address"));
    }
    if crate::services::rate_limit_service::cloudflare()
        .iter()
        .any(|c| crate::services::rate_limit_service::overlaps(c, &cidr))
    {
        return Err(format!(
            "{ip} is Cloudflare's: blocking it would block its visitors"
        ));
    }
    Ok(address.to_string())
}

/// How long a block lasts: a day, a week or thirty days.
pub fn duration(days: u32) -> Result<String, String> {
    match days {
        1 | 7 | 30 => Ok(format!("{}h", days * 24)),
        _ => Err("a day, a week or thirty days".to_owned()),
    }
}

/// Blocks `ip` for `days` (`reason`: letters, digits, spaces and `.,:-`).
pub async fn block(docker: &Docker, ip: &str, days: u32, reason: &str) -> AppResult<()> {
    let ip = blockable(ip).map_err(|e| rustcms::ValidationErrors::field("ip", e))?;
    let duration = duration(days).map_err(|e| rustcms::ValidationErrors::field("days", e))?;
    let reason: String = reason
        .chars()
        .filter(|c| c.is_alphanumeric() || " .,:-".contains(*c))
        .take(100)
        .collect();
    let reason = format!(
        "rdockpanel: {}",
        if reason.trim().is_empty() {
            "blocked"
        } else {
            reason.trim()
        }
    );
    cscli(
        docker,
        &[
            "decisions",
            "add",
            "--ip",
            &ip,
            "--duration",
            &duration,
            "--reason",
            &reason,
        ],
    )
    .await?
    .ok_or_else(|| AppError::BadRequest("CrowdSec is not running: turn the WAF on".into()))?;
    Ok(())
}

pub async fn unblock(docker: &Docker, ip: &str) -> AppResult<()> {
    let ip: std::net::IpAddr = ip
        .trim()
        .parse()
        .map_err(|_| rustcms::ValidationErrors::field("ip", "not an IP address"))?;
    cscli(docker, &["decisions", "delete", "--ip", &ip.to_string()])
        .await?
        .ok_or_else(|| AppError::BadRequest("CrowdSec is not running".into()))?;
    Ok(())
}

/// The blocklist: the addresses blocked by hand (with their reason and
/// what is left of their time), how many the community list has, and
/// whether CrowdSec reaches its community (sharing on).
pub async fn blocklist(docker: &Docker, settings: &Settings) -> AppResult<Value> {
    let Some(text) = cscli(
        docker,
        &[
            "decisions",
            "list",
            "-o",
            "json",
            "--origin",
            "cscli",
            "-l",
            "0",
        ],
    )
    .await?
    else {
        return Ok(json!({ "running": false, "blocked": [], "community": 0, "connected": false }));
    };
    let text = text.trim();
    let alerts: Vec<Value> = if text.is_empty() || text == "null" {
        vec![]
    } else {
        serde_json::from_str(text)
            .map_err(|_| AppError::internal("CrowdSec's decisions could not be read"))?
    };
    let blocked: Vec<Value> = alerts
        .iter()
        .flat_map(|a| {
            let reason = a["scenario"].as_str().unwrap_or_default().to_owned();
            a["decisions"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(move |d| {
                    json!({
                        "ip": d["value"],
                        "reason": reason.trim_start_matches("rdockpanel: "),
                        "left": d["duration"],
                    })
                })
        })
        .collect();
    let metrics: Value = cscli(docker, &["metrics", "show", "decisions", "-o", "json"])
        .await?
        .and_then(|t| serde_json::from_str(t.trim()).ok())
        .unwrap_or(Value::Null);
    let community: u64 = metrics["decisions"]
        .as_object()
        .into_iter()
        .flatten()
        .flat_map(|(_, by_origin)| by_origin.as_object().into_iter().flatten())
        .filter(|(origin, _)| matches!(origin.as_str(), "CAPI" | "lists"))
        .flat_map(|(_, by_type)| by_type.as_object().into_iter().flatten())
        .filter_map(|(_, n)| n.as_u64())
        .sum();
    let connected = settings.sharing
        && cscli(docker, &["capi", "status"])
            .await
            .map(|o| o.is_some())
            .unwrap_or(false);
    Ok(json!({
        "running": true,
        "blocked": blocked,
        "community": community,
        "connected": connected,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_addresses_not_cloudflares_are_blocked() {
        assert_eq!(blockable(" 45.12.3.4 ").unwrap(), "45.12.3.4");
        assert!(blockable("10.0.0.1").is_err());
        assert!(blockable("127.0.0.1").is_err());
        assert!(blockable("fd00::1").is_err());
        assert!(blockable("104.16.0.1").unwrap_err().contains("Cloudflare"));
        assert!(blockable("45.12.3.0/24").is_err());
        assert!(blockable("example.com").is_err());
        // Link-local, multicast, and private behind an IPv4-mapped address.
        assert!(blockable("fe80::1").is_err());
        assert!(blockable("ff02::1").is_err());
        assert!(blockable("::ffff:10.0.0.1").is_err());
        assert_eq!(blockable("::ffff:45.12.3.4").unwrap(), "45.12.3.4");
    }

    #[test]
    fn a_block_lasts_a_day_a_week_or_a_month() {
        assert_eq!(duration(1).unwrap(), "24h");
        assert_eq!(duration(7).unwrap(), "168h");
        assert_eq!(duration(30).unwrap(), "720h");
        assert!(duration(2).is_err());
    }

    #[test]
    fn outdated_collections_are_counted() {
        let check = RulesCheck {
            collections: vec![
                Versions {
                    name: "a".into(),
                    local: Some("1".into()),
                    latest: Some("2".into()),
                    ..Default::default()
                },
                Versions {
                    name: "b".into(),
                    local: Some("1".into()),
                    latest: Some("1".into()),
                    ..Default::default()
                },
                Versions {
                    name: "c".into(),
                    local: Some("1".into()),
                    latest: None,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(check.outdated(), 1);
    }
}
