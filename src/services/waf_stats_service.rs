//! The WAF's records, kept by the panel:
//! - statistics: every minute ([`sample`]), CrowdSec's AppSec counters
//!   (requests processed and blocked, both listeners) become per-hour rows
//!   of `waf_stats`. Its counters reset when it restarts (another
//!   container, or a counter lower than the last sample: the new value is
//!   the delta), and the panel's own test requests (the canary's, counted as
//!   they are sent) are left out;
//! - alerts: every minute ([`mirror`]), CrowdSec's new alerts are copied
//!   into `waf_alerts` (once each, by uuid), the canary's left out, the
//!   query's secrets masked; kept 30 days, the statistics 90.

use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration, DurationRound, Utc};
use rustcms::{AppError, AppResult};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::models::{panel_setting, waf_alert, waf_stat};

/// The canary's requests sent, and those blocked, since the panel started.
static CANARY_SENT: AtomicU64 = AtomicU64::new(0);
static CANARY_BLOCKED: AtomicU64 = AtomicU64::new(0);

/// Counts one of the canary's requests (reaching CrowdSec or not).
pub fn canary_sent(blocked: bool) {
    CANARY_SENT.fetch_add(1, Ordering::Relaxed);
    if blocked {
        CANARY_BLOCKED.fetch_add(1, Ordering::Relaxed);
    }
}

/// This run of the panel (its canary counters start at zero with it).
static EPOCH: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
});

const BASELINE_KEY: &str = "waf.metrics";
const CHECKPOINT_KEY: &str = "waf.alerts_since";
/// How long alerts and statistics are kept.
const ALERTS_DAYS: i64 = 30;
const STATS_DAYS: i64 = 90;
/// At most this many alerts copied a minute.
const MIRROR_LIMIT: usize = 1000;

/// The counters at the last sample.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Baseline {
    pub container: String,
    pub processed: u64,
    pub blocked: u64,
    pub canary_sent: u64,
    pub canary_blocked: u64,
    /// The panel's run the canary counters are from.
    #[serde(default)]
    pub epoch: String,
}

/// What a sample adds to its hour: (checked, blocked).
pub fn delta(before: &Baseline, now: &Baseline) -> (i64, i64) {
    let same = before.container == now.container;
    let grow = |was: u64, is: u64| if same && is >= was { is - was } else { is };
    // The canary's counters are the panel's: another run of the panel
    // started them from zero, whatever CrowdSec did.
    let fresh = before.epoch != now.epoch;
    let canary = |was: u64, is: u64| if !fresh && is >= was { is - was } else { is };
    let processed = grow(before.processed, now.processed);
    let blocked = grow(before.blocked, now.blocked);
    let sent = canary(before.canary_sent, now.canary_sent);
    let sent_blocked = canary(before.canary_blocked, now.canary_blocked);
    (
        i64::try_from(processed.saturating_sub(sent)).unwrap_or(i64::MAX),
        i64::try_from(blocked.saturating_sub(sent_blocked)).unwrap_or(i64::MAX),
    )
}

/// CrowdSec's AppSec counters (`cscli metrics show appsec -o json`): the
/// requests processed and blocked, both listeners together.
pub fn counters(metrics: &Value) -> (u64, u64) {
    let engines = metrics["appsec-engine"].as_object().into_iter().flatten();
    engines.fold((0, 0), |(p, b), (_, e)| {
        (
            p + e["processed"].as_u64().unwrap_or(0),
            b + e["blocked"].as_u64().unwrap_or(0),
        )
    })
}

/// One minute's sample: the counters' growth since the last one added to
/// this hour's row, with the new baseline, in one transaction.
pub async fn sample(
    db: &sea_orm::DatabaseConnection,
    container: &str,
    metrics: &Value,
) -> AppResult<()> {
    let (processed, blocked) = counters(metrics);
    let now = Baseline {
        container: container.to_owned(),
        processed,
        blocked,
        canary_sent: CANARY_SENT.load(Ordering::Relaxed),
        canary_blocked: CANARY_BLOCKED.load(Ordering::Relaxed),
        epoch: EPOCH.clone(),
    };
    let txn = rustcms::db::begin_write(db).await?;
    let before: Option<Baseline> = panel_setting::get(&txn, BASELINE_KEY)
        .await?
        .and_then(|t| serde_json::from_str(&t).ok());
    // The first sample only sets the baseline (its counters are older).
    if let Some(before) = before {
        let (checked, blocked) = delta(&before, &now);
        add(&txn, hour_of(Utc::now()), checked, blocked).await?;
    }
    let text = serde_json::to_string(&now).map_err(AppError::internal)?;
    panel_setting::set(&txn, BASELINE_KEY, &text).await?;
    txn.commit().await?;
    Ok(())
}

/// The hour `at` falls in.
pub fn hour_of(at: DateTime<Utc>) -> DateTime<Utc> {
    at.duration_trunc(Duration::hours(1)).unwrap_or(at)
}

async fn add<C: ConnectionTrait>(
    db: &C,
    hour: DateTime<Utc>,
    checked: i64,
    blocked: i64,
) -> AppResult<()> {
    use sea_orm::ActiveModelTrait;
    match waf_stat::Entity::find()
        .filter(waf_stat::Column::Hour.eq(hour))
        .lock_exclusive()
        .one(db)
        .await?
    {
        Some(row) => {
            let mut active: waf_stat::ActiveModel = row.clone().into();
            active.checked = Set(row.checked.saturating_add(checked));
            active.blocked = Set(row.blocked.saturating_add(blocked));
            active.samples = Set(row.samples.saturating_add(1));
            active.update(db).await?;
        }
        None => {
            waf_stat::ActiveModel {
                hour: Set(hour),
                checked: Set(checked),
                blocked: Set(blocked),
                samples: Set(1),
                ..Default::default()
            }
            .insert(db)
            .await?;
        }
    }
    Ok(())
}

/// The `--since` of the next copy: from the last alert copied (a minute
/// earlier, the copy skipping those it has), a day back at first, thirty at
/// most.
pub async fn since<C: ConnectionTrait>(db: &C) -> AppResult<String> {
    let last: Option<DateTime<Utc>> = panel_setting::get(db, CHECKPOINT_KEY)
        .await?
        .and_then(|t| DateTime::parse_from_rfc3339(&t).ok())
        .map(|t| t.with_timezone(&Utc));
    let minutes = match last {
        Some(at) => (Utc::now() - at).num_minutes() + 2,
        None => 24 * 60,
    };
    Ok(format!("{}m", minutes.clamp(2, ALERTS_DAYS * 24 * 60)))
}

/// Copies `alerts` (CrowdSec's `cscli alerts list -o json`) into
/// `waf_alerts`, those it has and the canary's left out; the checkpoint
/// moves to the newest one. How many were new.
pub async fn mirror(db: &sea_orm::DatabaseConnection, alerts: &[Value]) -> AppResult<usize> {
    let rows: Vec<waf_alert::ActiveModel> = alerts.iter().filter_map(row_of).collect();
    let newest = rows
        .iter()
        .filter_map(|r| match &r.at {
            sea_orm::ActiveValue::Set(at) => Some(*at),
            _ => None,
        })
        .max();
    let txn = rustcms::db::begin_write(db).await?;
    let mut added = 0;
    for row in rows {
        let uuid = match &row.uuid {
            sea_orm::ActiveValue::Set(u) => u.clone(),
            _ => continue,
        };
        let known = waf_alert::Entity::find()
            .filter(waf_alert::Column::Uuid.eq(&uuid))
            .one(&txn)
            .await?
            .is_some();
        if !known {
            waf_alert::Entity::insert(row).exec(&txn).await?;
            added += 1;
        }
    }
    if let Some(at) = newest {
        panel_setting::set(&txn, CHECKPOINT_KEY, &at.to_rfc3339()).await?;
    }
    // Rows copied before Detect rules were named (`native_rule:901340`):
    // named now, so that filters find them by the name shown.
    let old = waf_alert::Entity::find()
        .filter(waf_alert::Column::Rule.starts_with("native_rule:"))
        .limit(500)
        .all(&txn)
        .await?;
    for row in old {
        let rule = cut(&shown_rule(&row), 191);
        if rule != row.rule {
            let mut active: waf_alert::ActiveModel = row.into();
            active.rule = Set(rule);
            sea_orm::ActiveModelTrait::update(active, &txn).await?;
        }
    }
    txn.commit().await?;
    Ok(added)
}

/// At most this many a copy (the rest the next minute).
pub fn mirror_limit() -> usize {
    MIRROR_LIMIT
}

/// The row an alert becomes (`None`: the canary's, or one without a uuid).
pub fn row_of(alert: &Value) -> Option<waf_alert::ActiveModel> {
    let metas = |key: &str| -> Vec<String> {
        alert["events"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|e| e["meta"].as_array().cloned().unwrap_or_default())
            .filter(|m| m["key"] == key)
            .filter_map(|m| m["value"].as_str().map(str::to_owned))
            .collect()
    };
    let meta = |key: &str| metas(key).into_iter().next();
    // A ban made by hand (`cscli decisions add`, the IP lists tab) is an
    // alert too, with no request: not one a rule matched.
    if alert["kind"] == "cscli" {
        return None;
    }
    let uuid = alert["uuid"].as_str()?.to_owned();
    let host = meta("target_fqdn").or_else(|| meta("target_host"));
    if host.as_deref().is_some_and(|h| h.ends_with(".invalid")) {
        return None;
    }
    let scenario = alert["scenario"].as_str().unwrap_or_default();
    let detect = scenario == "rdp/appsec-detect";
    let rule_ids = meta("rule_ids").or_else(|| native_ids(&metas("rule_name")));
    let rule = if detect {
        detect_rule(&metas("rule_name"), rule_ids.as_deref()).unwrap_or_else(|| scenario.to_owned())
    } else {
        rule_name(scenario)
    };
    let at = alert["created_at"]
        .as_str()
        .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&Utc))?;
    let ip = alert["source"]["ip"]
        .as_str()
        .or(alert["source"]["value"].as_str())?
        .to_owned();
    Some(waf_alert::ActiveModel {
        uuid: Set(cut(&uuid, 64)),
        at: Set(at),
        ip: Set(cut(&ip, 64)),
        country: Set(alert["source"]["cn"]
            .as_str()
            .filter(|c| !c.is_empty())
            .map(|c| cut(c, 8))),
        asn: Set(alert["source"]["as_name"]
            .as_str()
            .filter(|c| !c.is_empty())
            .map(|c| cut(c, 191))),
        host: Set(host.map(|h| cut(&h, 255))),
        path: Set(meta("uri").or_else(|| meta("target_uri")).map(|p| mask(&p))),
        rule: Set(cut(&rule, 191)),
        rule_ids: Set(rule_ids.map(|r| cut(&r, 255))),
        blocked: Set(!detect),
        ..Default::default()
    })
}

/// A Block alert has no `rule_ids`: its events name the Core Rule Set's
/// rules that matched (`native_rule:942100`), gathered here as `[942100 …]`
/// so that an exception can name the one that caught it.
fn native_ids(names: &[String]) -> Option<String> {
    let mut ids: Vec<&str> = names
        .iter()
        .filter_map(|n| n.strip_prefix("native_rule:"))
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .collect();
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(*id));
    (!ids.is_empty()).then(|| format!("[{}]", ids.join(" ")))
}

/// A Detect alert's rule as people read it: the hub rule that matched
/// (`crowdsecurity/vpatch-env-access`), else the Core Rule Set's rule that
/// caught the request, with what it is about (`OWASP CRS: sql_injection
/// (942100)`); never its set-up or scoring rules (901…, 949…, 959…, 980…),
/// which every request matches.
pub fn detect_rule(names: &[String], ids: Option<&str>) -> Option<String> {
    if let Some(named) = names
        .iter()
        .find(|n| n.starts_with("crowdsecurity/") && *n != "crowdsecurity/appsec-vpatch")
    {
        return Some(named.clone());
    }
    let caught = ids
        .into_iter()
        .flat_map(|text| text.split(|c: char| !c.is_ascii_digit()))
        .chain(names.iter().filter_map(|n| n.strip_prefix("native_rule:")))
        .find(|id| {
            id.len() == 6
                && !["901", "949", "959", "980"]
                    .iter()
                    .any(|p| id.starts_with(p))
        });
    if let Some(id) = caught {
        return Some(format!("OWASP CRS: {} ({id})", crs_kind(id)));
    }
    names.first().cloned()
}

/// What a Core Rule Set rule is about, from its id's family.
fn crs_kind(id: &str) -> &'static str {
    match &id[..3] {
        "911" => "method",
        "913" => "scanner",
        "920" | "921" | "922" => "protocol",
        "930" => "lfi",
        "931" => "rfi",
        "932" => "rce",
        "933" => "php",
        "934" => "generic",
        "941" => "xss",
        "942" => "sql_injection",
        "943" => "session_fixation",
        "944" => "java",
        _ => "rule",
    }
}

/// An alert's rule as shown: rows copied before the panel named Detect
/// rules (`native_rule:901340`) named again from their rule ids.
pub fn shown_rule(row: &waf_alert::Model) -> String {
    if row.rule.starts_with("native_rule:") {
        detect_rule(std::slice::from_ref(&row.rule), row.rule_ids.as_deref())
            .unwrap_or_else(|| row.rule.clone())
    } else {
        row.rule.clone()
    }
}

/// A Block alert's rule as people read it: the Core Rule Set's anomaly
/// blocks (`anomaly score block: lfi: 5, anomaly: 5, `) become
/// `OWASP CRS: lfi`; a named rule stays as it is.
pub fn rule_name(scenario: &str) -> String {
    let Some(scores) = scenario.strip_prefix("anomaly score block:") else {
        return scenario.to_owned();
    };
    let kinds: Vec<&str> = scores
        .split(',')
        .filter_map(|part| part.split(':').next().map(str::trim))
        .filter(|kind| !kind.is_empty() && *kind != "anomaly")
        .collect();
    if kinds.is_empty() {
        "OWASP CRS".to_owned()
    } else {
        format!("OWASP CRS: {}", kinds.join(", "))
    }
}

fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// A query's name as the app reads it (`%74oken` is `token`, decoded
/// again while it changes: `%2574oken` too), lowercase.
fn plain(name: &str) -> String {
    let mut name = name.replace('+', " ");
    for _ in 0..4 {
        let decoded = percent_encoding::percent_decode_str(&name)
            .decode_utf8_lossy()
            .into_owned();
        if decoded == name {
            break;
        }
        name = decoded;
    }
    name.to_lowercase()
}

/// A request's path with its query's secrets masked (`token`, `password`,
/// `key`, `secret`, `code`, `auth`, `session`, `sig`), 512 characters at most.
pub fn mask(path: &str) -> String {
    const SECRET: [&str; 9] = [
        "token", "password", "passwd", "key", "secret", "code", "auth", "session", "sig",
    ];
    let masked = match path.split_once('?') {
        None => path.to_owned(),
        Some((base, query)) => {
            let params: Vec<String> = query
                .split('&')
                .map(|pair| match pair.split_once('=') {
                    Some((name, _)) if SECRET.iter().any(|s| plain(name).contains(s)) => {
                        format!("{name}=***")
                    }
                    _ => pair.to_owned(),
                })
                .collect();
            format!("{base}?{}", params.join("&"))
        }
    };
    cut(&masked, 512)
}

/// Rows older than their keeping deleted.
pub async fn prune<C: ConnectionTrait>(db: &C) -> AppResult<()> {
    let now = Utc::now();
    waf_alert::Entity::delete_many()
        .filter(waf_alert::Column::At.lt(now - Duration::days(ALERTS_DAYS)))
        .exec(db)
        .await?;
    waf_stat::Entity::delete_many()
        .filter(waf_stat::Column::Hour.lt(now - Duration::days(STATS_DAYS)))
        .exec(db)
        .await?;
    Ok(())
}

/// The overview of the last `hours`: totals, one point an hour (missing
/// hours have none: the page shows no data there), and the top rules,
/// addresses and domains of the alerts (alerts, not requests: Detect's
/// are grouped by address and path a minute).
pub async fn overview<C: ConnectionTrait>(db: &C, hours: i64) -> AppResult<Value> {
    let from = hour_of(Utc::now()) - Duration::hours(hours - 1);
    let stats = waf_stat::Entity::find()
        .filter(waf_stat::Column::Hour.gte(from))
        .order_by_asc(waf_stat::Column::Hour)
        .all(db)
        .await?;
    let checked: i64 = stats.iter().map(|s| s.checked).sum();
    let blocked: i64 = stats.iter().map(|s| s.blocked).sum();
    let alerts = waf_alert::Entity::find()
        .filter(waf_alert::Column::At.gte(from))
        .all(db)
        .await?;
    let recorded = alerts.iter().filter(|a| !a.blocked).count();
    let top = |key: fn(&waf_alert::Model) -> Option<String>| {
        let mut counts: std::collections::HashMap<String, usize> = Default::default();
        for a in &alerts {
            if let Some(k) = key(a) {
                *counts.entry(k).or_default() += 1;
            }
        }
        let mut list: Vec<(String, usize)> = counts.into_iter().collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        list.truncate(5);
        list.into_iter()
            .map(|(name, count)| json!({ "name": name, "count": count }))
            .collect::<Vec<_>>()
    };
    Ok(json!({
        "from": from,
        "checked": checked,
        "blocked": blocked,
        "recorded": recorded,
        "series": stats.iter().map(|s| json!({
            "hour": s.hour, "checked": s.checked, "blocked": s.blocked, "samples": s.samples,
        })).collect::<Vec<_>>(),
        "top_rules": top(|a| Some(shown_rule(a))),
        "top_ips": top(|a| Some(a.ip.clone())),
        "top_hosts": top(|a| a.host.clone()),
    }))
}

/// A page of alerts (25, newest first) matching the filters.
#[derive(Debug, Default, Deserialize)]
pub struct AlertQuery {
    pub page: Option<u64>,
    pub host: Option<String>,
    pub ip: Option<String>,
    pub rule: Option<String>,
    /// `blocked` or `recorded`.
    pub result: Option<String>,
    /// Hours back (24 by default, 30 days at most).
    pub hours: Option<i64>,
}

pub async fn alerts<C: ConnectionTrait>(db: &C, query: &AlertQuery) -> AppResult<Value> {
    use sea_orm::PaginatorTrait;
    let hours = query.hours.unwrap_or(24).clamp(1, ALERTS_DAYS * 24);
    let mut select = waf_alert::Entity::find()
        .filter(waf_alert::Column::At.gte(Utc::now() - Duration::hours(hours)));
    if let Some(host) = query.host.as_deref().filter(|h| !h.is_empty()) {
        select = select.filter(waf_alert::Column::Host.eq(host));
    }
    if let Some(ip) = query.ip.as_deref().filter(|h| !h.is_empty()) {
        select = select.filter(waf_alert::Column::Ip.eq(ip));
    }
    if let Some(rule) = query.rule.as_deref().filter(|h| !h.is_empty()) {
        // A rule's name or one of its ids (942100).
        select = select.filter(
            sea_orm::Condition::any()
                .add(waf_alert::Column::Rule.contains(rule))
                .add(waf_alert::Column::RuleIds.contains(rule)),
        );
    }
    match query.result.as_deref() {
        Some("blocked") => select = select.filter(waf_alert::Column::Blocked.eq(true)),
        Some("recorded") => select = select.filter(waf_alert::Column::Blocked.eq(false)),
        _ => {}
    }
    let pages = select
        .order_by_desc(waf_alert::Column::At)
        .order_by_desc(waf_alert::Column::Id)
        .paginate(db, 25);
    let total = pages.num_items().await?;
    let page = query.page.unwrap_or(1).max(1);
    let rows: Vec<Value> = pages
        .fetch_page(page - 1)
        .await?
        .into_iter()
        .map(|row| {
            let rule = shown_rule(&row);
            let mut value = serde_json::to_value(&row).unwrap_or(Value::Null);
            value["rule"] = json!(rule);
            value
        })
        .collect();
    Ok(json!({ "alerts": rows, "total": total, "page": page, "per_page": 25 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(
        container: &str,
        processed: u64,
        blocked: u64,
        sent: u64,
        sent_blocked: u64,
    ) -> Baseline {
        Baseline {
            container: container.to_owned(),
            processed,
            blocked,
            canary_sent: sent,
            canary_blocked: sent_blocked,
            epoch: "run-1".to_owned(),
        }
    }

    #[test]
    fn a_sample_adds_what_grew_without_the_canary() {
        assert_eq!(
            delta(&base("a", 100, 10, 8, 4), &base("a", 150, 13, 12, 6)),
            (46, 1)
        );
        // CrowdSec restarted (another container, or counters lower): the
        // new counters are all new.
        assert_eq!(
            delta(&base("a", 100, 10, 0, 0), &base("b", 30, 2, 0, 0)),
            (30, 2)
        );
        assert_eq!(
            delta(&base("a", 100, 10, 0, 0), &base("a", 20, 1, 0, 0)),
            (20, 1)
        );
        // The panel restarted (its canary counters reset).
        assert_eq!(
            delta(&base("a", 100, 10, 50, 20), &base("a", 110, 12, 4, 2)),
            (6, 0)
        );
        // The panel restarted and sent more than before: its counters are
        // still all new (3 canary requests, not 3 - 2).
        let restarted = Baseline {
            epoch: "run-2".to_owned(),
            ..base("a", 110, 12, 3, 1)
        };
        assert_eq!(delta(&base("a", 100, 10, 2, 0), &restarted), (7, 1));
        // Never below zero (canary requests CrowdSec never saw).
        assert_eq!(
            delta(&base("a", 100, 10, 0, 0), &base("a", 101, 10, 4, 2)),
            (0, 0)
        );
    }

    #[test]
    fn the_counters_sum_both_listeners() {
        let metrics = json!({"appsec-engine": {
            "0.0.0.0:7422/": {"blocked": 18, "processed": 27},
            "0.0.0.0:7423/": {"processed": 15},
        }});
        assert_eq!(counters(&metrics), (42, 18));
    }

    #[test]
    fn the_core_rule_sets_blocks_are_named_by_what_they_caught() {
        assert_eq!(
            rule_name("anomaly score block: lfi: 5, anomaly: 5, "),
            "OWASP CRS: lfi"
        );
        assert_eq!(
            rule_name("anomaly score block: sql_injection: 10, xss: 5, anomaly: 15, "),
            "OWASP CRS: sql_injection, xss"
        );
        assert_eq!(
            rule_name("crowdsecurity/vpatch-env-access"),
            "crowdsecurity/vpatch-env-access"
        );
    }

    #[test]
    fn a_querys_secrets_are_masked() {
        assert_eq!(
            mask("/a?id=1&token=abc&Api_Key=x"),
            "/a?id=1&token=***&Api_Key=***"
        );
        assert_eq!(mask("/plain"), "/plain");
        // Encoded names are what the app reads.
        assert_eq!(
            mask("/?%74oken=abc&%2550assword=x"),
            "/?%74oken=***&%2550assword=***"
        );
        assert_eq!(mask(&"/x".repeat(400)).chars().count(), 512);
    }

    #[test]
    fn an_alert_becomes_a_row_but_the_canarys() {
        let alert = |host: &str, scenario: &str| {
            json!({
                "uuid": "u-1", "created_at": "2026-11-18T01:48:59Z", "scenario": scenario,
                "source": {"ip": "116.108.17.218", "cn": "VN", "as_name": "Viettel Group"},
                "events": [{"meta": [
                    {"key": "target_host", "value": host},
                    {"key": "target_uri", "value": "/?id=1&password=x"},
                    {"key": "rule_name", "value": "native_rule:942100"},
                    {"key": "rule_ids", "value": "[942100 949110]"},
                ]}],
            })
        };
        let row = row_of(&alert("shop.example.com", "rdp/appsec-detect")).unwrap();
        assert_eq!(
            row.rule,
            Set("OWASP CRS: sql_injection (942100)".to_owned())
        );
        assert_eq!(row.path, Set(Some("/?id=1&password=***".to_owned())));
        assert_eq!(row.blocked, Set(false));
        assert_eq!(row.country, Set(Some("VN".to_owned())));
        assert!(row_of(&alert("rdp-waf-canary-detect.invalid", "rdp/appsec-detect")).is_none());
        // A ban made by hand is no request a rule matched.
        let mut ban = alert("shop.example.com", "rdockpanel: spam");
        ban["kind"] = json!("cscli");
        ban["events"] = json!(null);
        assert!(row_of(&ban).is_none());
    }

    #[test]
    fn a_detect_alert_is_named_by_the_rule_that_caught_it() {
        // As a real server recorded them: the first event names the set-up
        // rule every request matches.
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            detect_rule(
                &names(&["native_rule:901340"]),
                Some("[901340 942100 949110 980170]")
            ),
            Some("OWASP CRS: sql_injection (942100)".to_owned())
        );
        assert_eq!(
            detect_rule(
                &names(&["native_rule:901340"]),
                Some("[901340 930130 373950650 980170]")
            ),
            Some("OWASP CRS: lfi (930130)".to_owned())
        );
        // A hub rule's name wins.
        assert_eq!(
            detect_rule(
                &names(&["native_rule:901340", "crowdsecurity/vpatch-env-access"]),
                Some("[901340 930130 373950650]")
            ),
            Some("crowdsecurity/vpatch-env-access".to_owned())
        );
        // Rows copied before: named again when shown.
        let old = waf_alert::Model {
            id: 1,
            uuid: "u".to_owned(),
            at: Utc::now(),
            ip: "116.108.17.218".to_owned(),
            country: None,
            asn: None,
            host: None,
            path: None,
            rule: "native_rule:901340".to_owned(),
            rule_ids: Some("[901340 942100 949110 980170]".to_owned()),
            blocked: false,
        };
        assert_eq!(shown_rule(&old), "OWASP CRS: sql_injection (942100)");
    }

    #[test]
    fn a_block_alert_keeps_the_rules_its_events_name() {
        let alert = json!({
            "uuid": "u-2", "created_at": "2026-11-18T01:48:59Z",
            "scenario": "anomaly score block: sql_injection: 10, anomaly: 10, ",
            "source": {"ip": "116.108.17.218"},
            "events": [
                {"meta": [{"key": "target_fqdn", "value": "shop.example.com"}, {"key": "rule_name", "value": "native_rule:942100"}]},
                {"meta": [{"key": "rule_name", "value": "native_rule:942190"}]},
                {"meta": [{"key": "rule_name", "value": "native_rule:942100"}]},
                {"meta": [{"key": "rule_name", "value": "native_rule:949110"}]},
            ],
        });
        let row = row_of(&alert).unwrap();
        assert_eq!(row.rule, Set("OWASP CRS: sql_injection".to_owned()));
        assert_eq!(row.rule_ids, Set(Some("[942100 942190 949110]".to_owned())));
        assert_eq!(row.blocked, Set(true));
        // A hub rule's Block alert: the scenario is the rule, the request is
        // `target_fqdn` and `uri`.
        let vpatch = json!({
            "uuid": "u-3", "created_at": "2026-11-18T01:48:59Z",
            "scenario": "crowdsecurity/vpatch-env-access",
            "source": {"ip": "116.108.17.218"},
            "events": [{"meta": [
                {"key": "target_fqdn", "value": "shop.example.com"},
                {"key": "uri", "value": "/.env"},
                {"key": "rule_name", "value": "crowdsecurity/vpatch-env-access"},
            ]}],
        });
        let row = row_of(&vpatch).unwrap();
        assert_eq!(row.rule, Set("crowdsecurity/vpatch-env-access".to_owned()));
        assert_eq!(
            (row.host, row.path),
            (
                Set(Some("shop.example.com".to_owned())),
                Set(Some("/.env".to_owned()))
            )
        );
        assert_eq!(row.rule_ids, Set(None));
    }
}
