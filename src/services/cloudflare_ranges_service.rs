//! Cloudflare's address ranges, kept up to date. They decide who may name
//! the visitor (`X-Forwarded-For`): Traefik's public entrypoints trust them,
//! the rate limit counts their visitors by the address they name, the WAF
//! reads the visitor through them, and no trusted or exempt address may be
//! one of theirs.
//!
//! - The ranges in use are the panel's built-in list
//!   ([`crate::services::rate_limit_service::CLOUDFLARE`]) until another is
//!   applied; [`ranges`] reads them (a cache, loaded at boot).
//! - [`check`] (daily, or the WAF page's button) asks Cloudflare's API and
//!   validates the answer strictly ([`validate`]). A list that only drops
//!   ranges is applied at once (trusting fewer is safe); one that adds
//!   ranges is offered, and an administrator applies it.
//! - Applying ([`apply`]) is an action: the list stored (audited), the
//!   cache, Traefik's entrypoints (restarted when they change), every
//!   route, and a file the installer reads (`.cloudflare-ips` in Traefik's
//!   directory: Traefik ignores a file that is not YAML or TOML).

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::sync::RwLock;

use chrono::{DateTime, Utc};
use rustcms::audit::{self, AuditEntry};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::models::panel_setting;
use crate::services::action_service::Initiator;
use crate::services::firewall_rules::Cidr;

/// Where the list is asked for.
pub const SOURCE: &str = "https://api.cloudflare.com/client/v4/ips";
const APPLIED_KEY: &str = "cloudflare.ranges";
const CHECK_KEY: &str = "cloudflare.check";
/// The file the installer reads, in Traefik's dynamic directory.
pub const FILE: &str = ".cloudflare-ips";
pub const ACTION: &str = "cloudflare.apply";

static CURRENT: RwLock<Option<Vec<String>>> = RwLock::new(None);

/// The ranges in use (the built-in list until another is applied).
pub fn ranges() -> Vec<String> {
    CURRENT
        .read()
        .ok()
        .and_then(|c| c.clone())
        .unwrap_or_else(builtin)
}

fn builtin() -> Vec<String> {
    crate::services::rate_limit_service::CLOUDFLARE
        .iter()
        .map(|c| (*c).to_owned())
        .collect()
}

/// The list applied (stored).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Applied {
    pub ranges: Vec<String>,
    pub applied_at: Option<DateTime<Utc>>,
    /// `builtin` or `cloudflare`.
    pub source: String,
    /// Stored but not yet in Traefik, the routes and the WAF (the spread
    /// failed or was cut short): spread again at boot and at each check.
    #[serde(default)]
    pub pending: bool,
}

/// The last check.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    pub checked_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Ranges Cloudflare added, waiting for an administrator.
    #[serde(default)]
    pub offered: Vec<String>,
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
}

pub async fn applied<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<Applied> {
    Ok(panel_setting::get(db, APPLIED_KEY)
        .await?
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| Applied {
            ranges: builtin(),
            applied_at: None,
            source: "builtin".to_owned(),
            pending: false,
        }))
}

pub async fn last_check<C: sea_orm::ConnectionTrait>(db: &C) -> AppResult<Check> {
    Ok(panel_setting::get(db, CHECK_KEY)
        .await?
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default())
}

/// At boot, before any route is written: the cache from the stored list.
pub async fn load<C: sea_orm::ConnectionTrait>(db: &C) {
    match applied(db).await {
        Ok(list) if list.source != "builtin" => set_current(list.ranges),
        Ok(_) => {}
        Err(err) => tracing::warn!(%err, "Cloudflare's ranges not read: the built-in ones"),
    }
}

fn set_current(list: Vec<String>) {
    if let Ok(mut current) = CURRENT.write() {
        *current = Some(list);
    }
}

/// Not a public network: private, loopback, link-local, shared (CGNAT),
/// multicast, documentation, unspecified.
fn private(cidr: &Cidr) -> bool {
    !public(cidr.address)
}

/// A public address: none of the private, loopback, link-local, shared
/// (CGNAT), multicast, documentation or unspecified ones; an IPv4-mapped
/// IPv6 address (`::ffff:10.0.0.1`) is judged as its IPv4 one.
pub fn public(address: IpAddr) -> bool {
    let private = match address {
        IpAddr::V6(v6) if v6.to_ipv4_mapped().is_some() => {
            return v6.to_ipv4_mapped().is_some_and(|v4| public(IpAddr::V4(v4)));
        }
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0)
                || (a == 198 && (18..20).contains(&b))
                || (a == 198 && b == 51 && v4.octets()[2] == 100)
                || (a == 203 && b == 0 && v4.octets()[2] == 113)
                || a >= 240
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first == 0x2001 && v6.segments()[1] == 0x0db8)
        }
    };
    !private
}

/// How many IPv4 addresses `list` covers.
fn v4_space(list: &[Cidr]) -> u64 {
    list.iter()
        .filter(|c| c.is_v4())
        .map(|c| 1u64 << (32 - u32::from(c.prefix)))
        .sum()
}

/// Cloudflare's answer, checked against the list in use: canonical,
/// deduplicated and sorted, or why it is refused. Every range a public
/// network no wider than /12 (IPv4) or /29 (IPv6); 10 to 100 IPv4 ranges
/// and 3 to 100 IPv6 ones; the IPv4 space grown by 25% at most; at most
/// half the ranges changed (added and removed, against the list in use).
pub fn validate(v4: &[String], v6: &[String], current: &[String]) -> Result<Vec<String>, String> {
    let mut set = BTreeSet::new();
    for (text, want_v4) in v4
        .iter()
        .map(|t| (t, true))
        .chain(v6.iter().map(|t| (t, false)))
    {
        let cidr = Cidr::parse(text)?;
        if cidr.is_v4() != want_v4 {
            return Err(format!("{text} is in the wrong list"));
        }
        if private(&cidr) {
            return Err(format!("{text} is not a public network"));
        }
        let narrowest = if cidr.is_v4() { 12 } else { 29 };
        if cidr.prefix < narrowest {
            return Err(format!("{text} is wider than /{narrowest}"));
        }
        set.insert(cidr);
    }
    let list: Vec<Cidr> = set.into_iter().collect();
    let fours = list.iter().filter(|c| c.is_v4()).count();
    let sixes = list.len() - fours;
    if !(10..=100).contains(&fours) || !(3..=100).contains(&sixes) {
        return Err(format!(
            "{fours} IPv4 and {sixes} IPv6 ranges: not Cloudflare's list"
        ));
    }
    let now: Vec<Cidr> = current.iter().filter_map(|c| Cidr::parse(c).ok()).collect();
    let space = v4_space(&now);
    if space > 0 && v4_space(&list) > space + space / 4 {
        return Err("the IPv4 ranges grew by more than a quarter".to_owned());
    }
    let changed = list.iter().filter(|c| !now.contains(c)).count()
        + now.iter().filter(|c| !list.contains(c)).count();
    if !now.is_empty() && changed * 2 > now.len() {
        return Err(format!("{changed} of {} ranges changed at once", now.len()));
    }
    Ok(list.iter().map(ToString::to_string).collect())
}

/// What `next` adds to and removes from `current`.
pub fn diff(current: &[String], next: &[String]) -> (Vec<String>, Vec<String>) {
    let added = next
        .iter()
        .filter(|c| !current.contains(c))
        .cloned()
        .collect();
    let removed = current
        .iter()
        .filter(|c| !next.contains(c))
        .cloned()
        .collect();
    (added, removed)
}

/// Asks Cloudflare, validates, and applies a list that only drops ranges;
/// one that adds is offered. The check stored either way.
pub async fn check(app: &AppState) -> AppResult<Check> {
    resume(app).await;
    let current = applied(app.db()).await?.ranges;
    let outcome = fetch()
        .await
        .and_then(|(v4, v6)| validate(&v4, &v6, &current));
    let mut check = Check {
        checked_at: Some(Utc::now()),
        ..Check::default()
    };
    match outcome {
        Err(err) => check.error = Some(err),
        Ok(next) => {
            let (added, removed) = diff(&current, &next);
            check.added = added.clone();
            check.removed = removed.clone();
            if added.is_empty() && !removed.is_empty() {
                // Trusting fewer is safe: at once.
                apply(app, next, None).await?;
            } else if !added.is_empty() {
                check.offered = next;
                crate::services::notify_service::notify(
                    app,
                    crate::services::notify_service::Event::CloudflareRanges { added: added.len() },
                );
            }
        }
    }
    let text = serde_json::to_string(&check).map_err(AppError::internal)?;
    panel_setting::set(app.db(), CHECK_KEY, &text).await?;
    Ok(check)
}

/// Cloudflare's lists (IPv4, IPv6).
async fn fetch() -> Result<(Vec<String>, Vec<String>), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("RDockPanel")
        .build()
        .map_err(|e| e.to_string())?;
    let body: serde_json::Value = client
        .get(SOURCE)
        .send()
        .await
        .map_err(|e| format!("Cloudflare did not answer: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Cloudflare answered {e}"))?
        .json()
        .await
        .map_err(|e| format!("Cloudflare's answer could not be read: {e}"))?;
    let list = |key: &str| -> Vec<String> {
        body["result"][key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    };
    if body["success"] != json!(true) {
        return Err("Cloudflare's answer says it failed".to_owned());
    }
    Ok((list("ipv4_cidrs"), list("ipv6_cidrs")))
}

/// Applies `ranges` (validated): stored and audited (as the system when
/// no user asked), the cache, the installer's file, Traefik's entrypoints
/// (restarted when they change) and every route. An action logs it; the
/// list stays `pending` until all of it is done.
pub async fn apply(app: &AppState, ranges: Vec<String>, by: Option<&Initiator>) -> AppResult<()> {
    let system = Initiator::system();
    let by = by.unwrap_or(&system);
    let txn = rustcms::db::begin_write(app.db()).await?;
    let before = applied(&txn).await?;
    let list = Applied {
        ranges: ranges.clone(),
        applied_at: Some(Utc::now()),
        source: "cloudflare".to_owned(),
        pending: true,
    };
    let text = serde_json::to_string(&list).map_err(AppError::internal)?;
    panel_setting::set(&txn, APPLIED_KEY, &text).await?;
    let (added, removed) = diff(&before.ranges, &ranges);
    audit::record(
        &txn,
        &by.audit,
        AuditEntry::new("cloudflare.ranges", "panel_settings")
            .key(APPLIED_KEY)
            .meta(json!({ "added": added, "removed": removed })),
    )
    .await?;
    let row = crate::services::action_service::start(
        &txn,
        by,
        ACTION,
        "Cloudflare",
        crate::models::action::RUNNING,
    )
    .await?;
    // The offer is answered.
    let mut check = last_check(&txn).await?;
    check.offered.clear();
    let check_text = serde_json::to_string(&check).map_err(AppError::internal)?;
    panel_setting::set(&txn, CHECK_KEY, &check_text).await?;
    txn.commit().await?;
    let outcome = spread_and_mark(app).await;
    let error = outcome.as_ref().err().map(ToString::to_string);
    crate::services::action_service::finish(
        app.db(),
        row,
        error.clone(),
        Some(&format!(
            "{} ranges in use.\n{}",
            ranges.len(),
            error
                .clone()
                .unwrap_or_else(|| "Traefik and the routes updated.".to_owned())
        )),
    )
    .await?;
    outcome
}

/// A stored list not spread yet (see [`Applied::pending`]): spread again.
pub async fn resume(app: &AppState) {
    if let Err(err) = spread_and_mark(app).await {
        tracing::warn!(%err, "Cloudflare's ranges still not applied");
    }
}

/// One spread at a time.
static SPREADING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The stored list, if pending, spread (read under [`SPREADING`]: the
/// latest list is the one spread last, never an older one after it), then
/// no longer pending if it is still the stored one.
async fn spread_and_mark(app: &AppState) -> AppResult<()> {
    let _one = SPREADING.lock().await;
    let stored = applied(app.db()).await?;
    if !stored.pending {
        return Ok(());
    }
    let ranges = stored.ranges;
    set_current(ranges.clone());
    spread(app, &ranges).await?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    let default = serde_json::to_string(&Applied::default()).map_err(AppError::internal)?;
    let text = panel_setting::lock(&txn, APPLIED_KEY, &default).await?;
    if let Ok(mut list) = serde_json::from_str::<Applied>(&text)
        && list.pending
        && list.ranges == ranges
    {
        list.pending = false;
        let text = serde_json::to_string(&list).map_err(AppError::internal)?;
        panel_setting::set(&txn, APPLIED_KEY, &text).await?;
    }
    txn.commit().await?;
    Ok(())
}

/// Where the ranges are used, given the new list: the installer's file,
/// Traefik's entrypoints, the routes, the WAF's definitions.
async fn spread(app: &AppState, ranges: &[String]) -> AppResult<()> {
    let (Some(docker), Some(traefik)) = (
        app.service::<crate::services::docker_service::Docker>(),
        app.service::<crate::services::traefik_service::Traefik>(),
    ) else {
        return Ok(());
    };
    let dir = traefik.dir.clone();
    let text = format!("{}\n", ranges.join("\n"));
    tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        crate::services::certificate_service::write_private(&dir, FILE, text.as_bytes())
    })
    .await
    .map_err(AppError::internal)?
    .map_err(AppError::internal)?;
    if !traefik.service.is_empty() {
        crate::services::rate_limit_service::trust_now(&docker, &traefik)
            .await
            .map_err(|e| AppError::internal(e.to_string()))?;
    }
    let options = crate::services::deployment_service::deploy_options(0);
    crate::services::traefik_service::sync_all(app.db(), &traefik, &options).await?;
    crate::services::waf_docker_service::refresh_definitions(app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> Vec<String> {
        builtin()
    }

    fn split(list: &[String]) -> (Vec<String>, Vec<String>) {
        list.iter().cloned().partition(|c| !c.contains(':'))
    }

    #[test]
    fn cloudflares_own_list_passes() {
        let (v4, v6) = split(&now());
        let list = validate(&v4, &v6, &now()).unwrap();
        assert_eq!(list.len(), now().len());
    }

    #[test]
    fn a_strange_list_is_refused() {
        let (mut v4, v6) = split(&now());
        // Too wide.
        let mut wide = v4.clone();
        wide[0] = "5.0.0.0/8".to_owned();
        assert!(validate(&wide, &v6, &now()).unwrap_err().contains("wider"));
        // Not public.
        let mut private = v4.clone();
        private[0] = "10.0.0.0/16".to_owned();
        assert!(
            validate(&private, &v6, &now())
                .unwrap_err()
                .contains("public")
        );
        // Too few.
        assert!(validate(&v4[..3], &v6, &now()).is_err());
        // In the wrong list.
        let mut wrong = v4.clone();
        wrong.push(v6[0].clone());
        assert!(validate(&wrong, &v6, &now()).is_err());
        // Half of it changed at once.
        let replaced: Vec<String> = (0..v4.len()).map(|i| format!("45.{i}.0.0/16")).collect();
        assert!(validate(&replaced, &v6, &now()).is_err());
        // The IPv4 space grown by more than a quarter (one /12 more).
        v4.push("5.0.0.0/12".to_owned());
        assert!(validate(&v4, &v6, &now()).unwrap_err().contains("quarter"));
    }

    #[test]
    fn a_list_is_canonical_and_its_changes_are_known() {
        let (mut v4, v6) = split(&now());
        v4.push(v4[0].clone());
        v4.push("198.51.99.7/24".to_owned());
        let list = validate(&v4, &v6, &now()).unwrap();
        assert!(list.contains(&"198.51.99.0/24".to_owned()));
        let (added, removed) = diff(&now(), &list);
        assert_eq!(added, vec!["198.51.99.0/24".to_owned()]);
        assert!(removed.is_empty());
    }
}
