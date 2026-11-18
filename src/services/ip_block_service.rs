//! Blocking an address, on the websites (the WAF's blocklist) or on the
//! whole server (the firewall), or both: the rules every writer follows
//! (the WAF page, the Firewall page, an alert's menu, the API).
//!
//! The two layers differ, and the page says so:
//! - **the WAF** answers 403 on the services' domains; it reads the
//!   visitor's own address through Cloudflare and the tunnel; one address,
//!   public, for a day, a week or thirty days (CrowdSec's decision);
//! - **the firewall** drops everything but SSH (every site, the panel
//!   included); it sees only who connects to the server (Cloudflare's
//!   address for a proxied visitor, nothing of the tunnel's); an address or
//!   a network, until removed or until its end. It is put on the server at
//!   once ([`firewall_service::apply_blocks`]).
//!
//! Never blocked, on either: Cloudflare's ranges (every visitor coming
//! through them would be), every address (`/0`), and the address the
//! administrator is using (they would lose the panel, or their sites).

use chrono::{DateTime, Duration, Utc};
use rustcms::audit::{self, AuditEntry};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, Set,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::models::firewall_blocked;
use crate::services::action_service::Initiator;
use crate::services::deploy_rules_service::Install;
use crate::services::docker_service::Docker;
use crate::services::firewall_rules::{self as rules, Cidr};
use crate::services::firewall_service::{self, BlocksApplied};
use crate::services::waf_service::{Mode, Settings};

/// The row locked by every writer of the firewall's list.
const LIST_LOCK: &str = "firewall.blocked";

/// A block asked for.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// An address, or a network for the firewall only.
    pub ip: String,
    /// On the websites (the WAF's blocklist).
    #[serde(default)]
    pub waf: bool,
    /// On the whole server (the firewall).
    #[serde(default)]
    pub firewall: bool,
    /// 1, 7 or 30; none: until removed (the firewall only).
    #[serde(default)]
    pub days: Option<u32>,
    #[serde(default)]
    pub reason: String,
}

/// Which layers to remove an address from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Removal {
    pub ip: String,
    #[serde(default)]
    pub waf: bool,
    #[serde(default)]
    pub firewall: bool,
}

/// What the request's `ip` is, checked for both layers: the network, and
/// for the WAF the single address it takes.
pub fn target(
    ip: &str,
    waf: bool,
    firewall: bool,
    own: Option<&str>,
) -> Result<Cidr, ValidationErrors> {
    if !waf && !firewall {
        return Err(ValidationErrors::field(
            "layers",
            "choose the websites (WAF), the server (Firewall) or both",
        ));
    }
    let text = ip.trim();
    let cidr = unmapped(Cidr::parse(text).map_err(|e| ValidationErrors::field("ip", e))?);
    if cidr.prefix == 0 {
        return Err(ValidationErrors::field("ip", "would block everyone"));
    }
    let single = cidr.prefix == if cidr.address.is_ipv4() { 32 } else { 128 };
    if waf && !single {
        return Err(ValidationErrors::field(
            "ip",
            "the WAF blocks one address at a time: choose the firewall alone for a network",
        ));
    }
    if crate::services::rate_limit_service::cloudflare()
        .iter()
        .any(|c| crate::services::rate_limit_service::overlaps(c, &cidr))
    {
        return Err(ValidationErrors::field(
            "ip",
            "is Cloudflare's: blocking it would block every visitor coming through Cloudflare",
        ));
    }
    if let Some(own) = own.and_then(|o| Cidr::parse(o).ok()).map(unmapped)
        && crate::services::rate_limit_service::overlaps(&own, &cidr)
    {
        return Err(ValidationErrors::field(
            "ip",
            format!("{own} is the address you are using: you would lose the panel or your sites"),
        ));
    }
    if waf {
        crate::services::waf_rules_service::blockable(&cidr.address.to_string())
            .map_err(|e| ValidationErrors::field("ip", e))?;
    }
    Ok(cidr)
}

/// An IPv4-mapped IPv6 network (`::ffff:45.12.3.4`) as the IPv4 one it is:
/// one family to compare, block and show.
pub fn unmapped(cidr: Cidr) -> Cidr {
    if let std::net::IpAddr::V6(v6) = cidr.address
        && let Some(v4) = v6.to_ipv4_mapped()
        && cidr.prefix >= 96
    {
        return Cidr {
            address: std::net::IpAddr::V4(v4),
            prefix: cidr.prefix - 96,
        };
    }
    cidr
}

/// The address the administrator acts from (`None`: from the panel itself).
fn own(by: &Initiator) -> Option<String> {
    by.audit.request.ip.clone()
}

/// Blocks `request.ip` where asked: the WAF's decision, the firewall's row
/// (audited with the change), then the firewall put on the server.
pub async fn block(
    app: &AppState,
    by: &Initiator,
    request: &Request,
    install: Install,
) -> AppResult<Value> {
    let own = own(by);
    let cidr = target(&request.ip, request.waf, request.firewall, own.as_deref())?;
    let ends = match request.days {
        None if request.waf => {
            return Err(ValidationErrors::field(
                "days",
                "the WAF blocks for a day, a week or thirty days",
            )
            .into());
        }
        None => None,
        Some(days @ (1 | 7 | 30)) => Some(Utc::now() + Duration::days(i64::from(days))),
        Some(_) => {
            return Err(ValidationErrors::field("days", "a day, a week or thirty days").into());
        }
    };
    let reason: String = request.reason.trim().chars().take(100).collect();
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))?;
    if request.waf && Settings::load(app.db()).await?.mode == Mode::Off {
        return Err(ValidationErrors::field("waf", "the WAF is off: turn it on first").into());
    }
    let mut out = json!({ "ip": display(&cidr) });
    // The WAF first: when CrowdSec refuses, nothing was changed at all.
    if request.waf {
        crate::services::waf_rules_service::block(
            &docker,
            &cidr.address.to_string(),
            request.days.unwrap_or(7),
            &reason,
        )
        .await?;
        audit::record_detached(
            app.db(),
            &by.audit,
            AuditEntry::new("waf.block", "waf")
                .key(cidr.address.to_string())
                .meta(json!({ "days": request.days, "reason": reason })),
        )
        .await;
        // Blocked only while the blocklist is on in Block mode; else saved.
        let settings = Settings::load(app.db()).await?;
        let active = settings.blocklist && settings.mode == Mode::Block;
        out["waf"] = json!(if active { "blocked" } else { "saved" });
    }
    if request.firewall {
        // Saved even where the firewall cannot run (it applies once it does).
        out["id"] = json!(save_firewall(app, by, &cidr, ends, &reason).await?);
        out["firewall"] = applied(app, &docker, by, install).await;
    }
    Ok(out)
}

/// The firewall's row: new, or the same network's end and note changed.
async fn save_firewall(
    app: &AppState,
    by: &Initiator,
    cidr: &Cidr,
    ends: Option<DateTime<Utc>>,
    note: &str,
) -> AppResult<i64> {
    let text = cidr.to_string();
    let note = (!note.is_empty()).then(|| note.to_owned());
    let txn = rustcms::db::begin_write(app.db()).await?;
    // One writer of the list at a time (a new row cannot be locked).
    crate::models::panel_setting::lock(&txn, LIST_LOCK, "").await?;
    let existing = firewall_blocked::Entity::find()
        .filter(firewall_blocked::Column::Cidr.eq(&text))
        .lock_exclusive()
        .one(&txn)
        .await?;
    let (row, event) = match existing {
        Some(row) => {
            let mut active: firewall_blocked::ActiveModel = row.into();
            active.expires_at = Set(ends);
            active.note = Set(note);
            (active.update(&txn).await?, "firewall.blocked.update")
        }
        None => {
            if firewall_blocked::Entity::find().count(&txn).await? as usize >= rules::BLOCKED_MAX {
                return Err(ValidationErrors::field(
                    "ip",
                    format!("at most {} addresses", rules::BLOCKED_MAX),
                )
                .into());
            }
            let row = firewall_blocked::ActiveModel {
                cidr: Set(text.clone()),
                note: Set(note),
                created_at: Set(Utc::now()),
                expires_at: Set(ends),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
            (row, "firewall.blocked.create")
        }
    };
    audit::record(
        &txn,
        &by.audit,
        AuditEntry::new(event, "firewall")
            .key(row.id.to_string())
            .meta(serde_json::to_value(&row).unwrap_or(Value::Null)),
    )
    .await?;
    txn.commit().await?;
    Ok(row.id)
}

/// The firewall put on the server, as the page reads it.
async fn applied(app: &AppState, docker: &Docker, by: &Initiator, install: Install) -> Value {
    match firewall_service::apply_blocks(app, docker, by, install).await {
        Ok(BlocksApplied::Done) => json!("applied"),
        Ok(BlocksApplied::Off) => json!("off"),
        Ok(BlocksApplied::Waiting(why)) => json!({ "waiting": why }),
        Err(err) => json!({ "waiting": err.to_string() }),
    }
}

/// Removes `removal.ip` where asked (audited), then the firewall put on the
/// server.
pub async fn unblock(
    app: &AppState,
    by: &Initiator,
    removal: &Removal,
    install: Install,
) -> AppResult<Value> {
    if !removal.waf && !removal.firewall {
        return Err(ValidationErrors::field(
            "layers",
            "choose the websites (WAF), the server (Firewall) or both",
        )
        .into());
    }
    let cidr =
        unmapped(Cidr::parse(removal.ip.trim()).map_err(|e| ValidationErrors::field("ip", e))?);
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))?;
    let mut out = json!({ "ip": display(&cidr) });
    // The WAF first: when CrowdSec refuses, nothing was changed at all.
    if removal.waf {
        crate::services::waf_rules_service::unblock(&docker, &cidr.address.to_string()).await?;
        audit::record_detached(
            app.db(),
            &by.audit,
            AuditEntry::new("waf.unblock", "waf").key(cidr.address.to_string()),
        )
        .await;
        out["waf"] = json!("unblocked");
    }
    if removal.firewall {
        let txn = rustcms::db::begin_write(app.db()).await?;
        // One writer of the list at a time (a new row cannot be locked).
        crate::models::panel_setting::lock(&txn, LIST_LOCK, "").await?;
        let rows = firewall_blocked::Entity::find()
            .filter(firewall_blocked::Column::Cidr.eq(cidr.to_string()))
            .lock_exclusive()
            .all(&txn)
            .await?;
        for row in rows {
            firewall_blocked::Entity::delete_by_id(row.id)
                .exec(&txn)
                .await?;
            audit::record(
                &txn,
                &by.audit,
                AuditEntry::new("firewall.blocked.delete", "firewall")
                    .key(row.id.to_string())
                    .meta(serde_json::to_value(&row).unwrap_or(Value::Null)),
            )
            .await?;
        }
        txn.commit().await?;
    }
    if removal.firewall {
        out["firewall"] = applied(app, &docker, by, install).await;
    }
    Ok(out)
}

/// The blocks whose end has passed: removed (audited as the system) and the
/// firewall put on the server without them. A line for the schedule's
/// history when some were.
pub async fn expire(app: &AppState, install: Install) -> AppResult<Option<String>> {
    let by = Initiator::system();
    let txn = rustcms::db::begin_write(app.db()).await?;
    // One writer of the list at a time (a new row cannot be locked).
    crate::models::panel_setting::lock(&txn, LIST_LOCK, "").await?;
    let ended = firewall_blocked::Entity::find()
        .filter(firewall_blocked::Column::ExpiresAt.lte(Utc::now()))
        .lock_exclusive()
        .all(&txn)
        .await?;
    for row in &ended {
        firewall_blocked::Entity::delete_by_id(row.id)
            .exec(&txn)
            .await?;
        audit::record(
            &txn,
            &by.audit,
            AuditEntry::new("firewall.blocked.expire", "firewall")
                .key(row.id.to_string())
                .meta(serde_json::to_value(row).unwrap_or(Value::Null)),
        )
        .await?;
    }
    txn.commit().await?;
    if ended.is_empty() {
        return Ok(None);
    }
    let Some(docker) = app.service::<Docker>() else {
        return Ok(None);
    };
    let state = applied(app, &docker, &by, install).await;
    Ok(Some(format!(
        "{} blocked addresses ended; firewall: {state}",
        ended.len()
    )))
}

/// Every blocked address, both layers together (one row an address), with
/// what each layer can do here and the administrator's own address.
pub async fn list(app: &AppState, own: Option<&str>, install: Install) -> AppResult<Value> {
    use std::collections::BTreeMap;
    let db = app.db();
    let settings = Settings::load(db).await?;
    let mut rows: BTreeMap<String, Value> = BTreeMap::new();
    let entry = |rows: &mut BTreeMap<String, Value>, key: &str| -> Value {
        rows.remove(key)
            .unwrap_or_else(|| json!({ "ip": key, "waf": null, "firewall": null }))
    };
    // The firewall's: in the database, on the server once applied.
    let on_server = firewall_service::blocked_on_server(db).await?;
    let now = Utc::now();
    for row in firewall_blocked::Entity::find().all(db).await? {
        if row.expires_at.is_some_and(|end| end <= now) {
            continue;
        }
        let key = Cidr::parse(&row.cidr).map_or(row.cidr.clone(), |c| display(&c));
        let mut item = entry(&mut rows, &key);
        item["firewall"] = json!({
            "id": row.id,
            "until": row.expires_at,
            "reason": row.note,
            "since": row.created_at,
            "on_server": on_server.contains(&row.cidr),
        });
        rows.insert(key, item);
    }
    // The WAF's (CrowdSec's decisions made by hand).
    let mut waf = json!({ "running": false, "community": 0, "connected": false });
    if settings.mode != Mode::Off
        && let Some(docker) = app.service::<Docker>()
    {
        waf = crate::services::waf_rules_service::blocklist(&docker, &settings).await?;
        for ban in waf["blocked"].as_array().cloned().unwrap_or_default() {
            let Some(ip) = ban["ip"].as_str() else {
                continue;
            };
            let key = Cidr::parse(ip).map_or(ip.to_owned(), |c| display(&c));
            let mut item = entry(&mut rows, &key);
            item["waf"] = json!({
                "until": ban["left"].as_str().and_then(go_duration).map(|d| now + d),
                "reason": ban["reason"],
            });
            rows.insert(key, item);
        }
    }
    let firewall_reason = firewall_service::unavailable(install);
    let enabled = firewall_reason.is_none() && firewall_service::enabled(db).await?;
    Ok(json!({
        "blocked": rows.into_values().collect::<Vec<_>>(),
        "you": own,
        "waf": {
            "enabled": settings.blocklist,
            "on": settings.mode != Mode::Off,
            "active": settings.blocklist && settings.mode == Mode::Block,
            "running": waf["running"].as_bool().unwrap_or(settings.mode != Mode::Off),
            "sharing": settings.sharing,
            "community": waf["community"],
            "connected": waf["connected"],
        },
        "firewall": {
            "available": firewall_reason.is_none(),
            "reason": firewall_reason,
            "enabled": enabled,
        },
    }))
}

/// A network as people write it: a single address without its `/32`.
fn display(cidr: &Cidr) -> String {
    let full = if cidr.address.is_ipv4() { 32 } else { 128 };
    if cidr.prefix == full {
        cidr.address.to_string()
    } else {
        cidr.to_string()
    }
}

/// A Go duration as CrowdSec prints it (`167h59m30.5s`, `-1m2s` once past).
pub fn go_duration(text: &str) -> Option<Duration> {
    let text = text.trim();
    if text.is_empty() || text.starts_with('-') {
        return None;
    }
    let mut total = 0f64;
    let mut number = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() || c == '.' {
            number.push(c);
            continue;
        }
        let value: f64 = number.parse().ok()?;
        number.clear();
        let unit = match c {
            'h' => 3600.0,
            'm' if chars.peek() == Some(&'s') => {
                chars.next();
                0.001
            }
            'm' => 60.0,
            's' => 1.0,
            _ => return None,
        };
        total += value * unit;
    }
    if !number.is_empty() {
        return None;
    }
    Some(Duration::milliseconds((total * 1000.0) as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crowdsecs_durations_are_read() {
        assert_eq!(
            go_duration("167h59m30s"),
            Some(Duration::seconds(167 * 3600 + 59 * 60 + 30))
        );
        assert_eq!(go_duration("1m2.5s"), Some(Duration::milliseconds(62_500)));
        assert_eq!(go_duration("-1m"), None);
        assert_eq!(go_duration("12"), None);
    }

    #[test]
    fn what_may_be_blocked_where() {
        // A network: the firewall's only.
        assert!(target("45.12.3.0/24", false, true, None).is_ok());
        assert!(target("45.12.3.0/24", true, true, None).is_err());
        // One address: both.
        assert_eq!(
            target("45.12.3.4", true, true, None).unwrap().to_string(),
            "45.12.3.4/32"
        );
        // Never Cloudflare's, nor everyone, nor no layer.
        assert!(target("104.16.0.1", false, true, None).is_err());
        assert!(target("0.0.0.0/0", false, true, None).is_err());
        assert!(target("45.12.3.4", false, false, None).is_err());
        // Never the administrator's own address (or a network holding it).
        assert!(target("45.12.3.4", true, false, Some("45.12.3.4")).is_err());
        assert!(target("45.12.0.0/16", false, true, Some("45.12.3.4")).is_err());
        // An IPv4-mapped address is the IPv4 one, on either side.
        assert!(target("45.12.3.4", false, true, Some("::ffff:45.12.3.4")).is_err());
        assert!(target("::ffff:45.12.3.4", true, false, Some("45.12.3.4")).is_err());
        assert_eq!(
            target("::ffff:45.12.3.4", false, true, None)
                .unwrap()
                .to_string(),
            "45.12.3.4/32"
        );
        // A private address: the firewall may (a LAN), the WAF may not.
        assert!(target("10.8.0.9", false, true, None).is_ok());
        assert!(target("10.8.0.9", true, false, None).is_err());
    }
}
