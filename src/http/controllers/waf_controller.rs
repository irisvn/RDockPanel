//! Settings → Security (the WAF): its settings, its state (the canary's),
//! repair, and CrowdSec's latest alerts — the panel's administrators only
//! (`system.manage`, routes/admin.rs). The canary's backend is public: it
//! answers a constant, nothing else.

use rustcms::audit::{self, AuditEntry, RequestMeta};

use crate::services::action_service::Initiator;
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::services::waf_docker_service::{self, Status};
use crate::services::waf_service::{Level, Mode, Settings};

/// The settings, the state, and how many services turned their WAF off.
pub async fn show(State(app): State<AppState>) -> AppResult<Json<Value>> {
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    let settings = Settings::load(app.db()).await?;
    let status = Status::load(app.db()).await?;
    let off = crate::models::service::Entity::find()
        .filter(crate::models::service::Column::Waf.eq("off"))
        .count(app.db())
        .await?;
    Ok(Json(
        json!({ "settings": settings, "status": status, "services_off": off }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsBody {
    pub mode: Mode,
    pub level: Level,
    #[serde(default)]
    pub allowlist: Vec<String>,
    #[serde(default)]
    pub sharing: bool,
    #[serde(default)]
    pub blocklist: bool,
}

/// Saves the settings (audited in the same transaction), then brings
/// CrowdSec, Traefik and the routes to them in the background: the page
/// follows the state ("starting", then "on" or "unavailable").
pub async fn update(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<SettingsBody>,
) -> AppResult<Json<Value>> {
    let mut allowlist: Vec<String> = body
        .allowlist
        .iter()
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty())
        .collect();
    allowlist.sort();
    allowlist.dedup();
    let mut settings = Settings {
        mode: body.mode,
        level: body.level,
        allowlist,
        sharing: body.sharing,
        blocklist: body.blocklist,
        ..Settings::default()
    };
    let problems = settings.problems();
    if !problems.is_empty() {
        let mut errors = ValidationErrors::new();
        for p in problems {
            errors.add("allowlist", p);
        }
        return Err(errors.into());
    }
    if settings.allowlist.len() > 200 {
        return Err(ValidationErrors::field("allowlist", "at most 200 addresses").into());
    }
    let by = Initiator::user(auth.principal(), request);
    let txn = rustcms::db::begin_write(app.db()).await?;
    let before = Settings::load_for_change(&txn).await?;
    // Kept as they are (their own pages change them).
    settings.exceptions = before.exceptions.clone();
    settings.rules = before.rules;
    settings.store(&txn).await?;
    let entry = AuditEntry::new("waf.settings", "panel_settings")
        .key(crate::services::waf_service::SETTINGS_KEY)
        .meta(json!({ "before": before, "after": settings }));
    audit::record(&txn, &by.audit, entry).await?;
    let (row, status) = announce(&txn, &by).await?;
    txn.commit().await?;
    waf_docker_service::reconcile_in_background_with(&app, false, Some(row));
    Ok(Json(json!({ "settings": settings, "status": status })))
}

/// Brings everything to the settings again (the plugin copied again,
/// CrowdSec and Traefik checked): after an "unavailable".
pub async fn repair(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let by = Initiator::user(auth.principal(), request);
    let txn = rustcms::db::begin_write(app.db()).await?;
    audit::record(
        &txn,
        &by.audit,
        AuditEntry::new("waf.repair", "panel_settings")
            .key(crate::services::waf_service::SETTINGS_KEY),
    )
    .await?;
    let (row, status) = announce(&txn, &by).await?;
    txn.commit().await?;
    waf_docker_service::reconcile_in_background_with(&app, false, Some(row));
    Ok(Json(json!({ "status": status })))
}

/// A new bouncer key (Traefik's for CrowdSec): CrowdSec, then Traefik, given
/// it (Traefik restarts: a few seconds' pause); the previous one goes once
/// the canary proves the new one. Audited; the WAF must be on.
pub async fn rotate_key(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    if Settings::load(app.db()).await?.mode == Mode::Off {
        return Err(AppError::BadRequest("the WAF is off: it has no key".into()));
    }
    let by = Initiator::user(auth.principal(), request);
    let txn = rustcms::db::begin_write(app.db()).await?;
    audit::record(
        &txn,
        &by.audit,
        AuditEntry::new("waf.rotate_key", "panel_settings")
            .key(crate::services::waf_service::SETTINGS_KEY),
    )
    .await?;
    let (row, status) = announce(&txn, &by).await?;
    txn.commit().await?;
    waf_docker_service::reconcile_in_background_with(&app, true, Some(row));
    Ok(Json(json!({ "status": status })))
}

/// The reconcile's action (running) and its first state, made in the
/// caller's transaction with its audit: the page follows them at once.
async fn announce(
    txn: &sea_orm::DatabaseTransaction,
    by: &Initiator,
) -> AppResult<(crate::models::action::Model, Status)> {
    let mode = Settings::load(txn).await?.mode;
    let row = crate::services::action_service::start(
        txn,
        by,
        waf_docker_service::ACTION,
        crate::services::waf_service::CROWDSEC,
        crate::models::action::RUNNING,
    )
    .await?;
    let status = if mode == Mode::Off {
        // Off once the reconcile has taken it off the routes.
        Status {
            action: Some(row.id),
            ..Status::now("stopping", None, false)
        }
    } else {
        Status::starting(1, chrono::Utc::now(), Some(row.id))
    };
    status.store(txn).await?;
    Ok((row, status))
}

/// The alerts the panel kept (30 days), 25 a page, filtered.
pub async fn alerts(
    State(app): State<AppState>,
    Query(query): Query<crate::services::waf_stats_service::AlertQuery>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        crate::services::waf_stats_service::alerts(app.db(), &query).await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct OverviewQuery {
    /// 24 (the default) or 168 (a week).
    pub hours: Option<i64>,
}

/// The overview: totals, hourly points, top rules, addresses and domains.
pub async fn overview(
    State(app): State<AppState>,
    Query(query): Query<OverviewQuery>,
) -> AppResult<Json<Value>> {
    let hours = match query.hours {
        Some(168) => 168,
        _ => 24,
    };
    Ok(Json(
        crate::services::waf_stats_service::overview(app.db(), hours).await?,
    ))
}

/// Cloudflare's ranges: those in use, the last check, those offered.
pub async fn cloudflare(State(app): State<AppState>) -> AppResult<Json<Value>> {
    use crate::services::cloudflare_ranges_service as cf;
    Ok(Json(json!({
        "applied": cf::applied(app.db()).await?,
        "check": cf::last_check(app.db()).await?,
        "source": cf::SOURCE,
    })))
}

/// Asks Cloudflare now (a list that only drops ranges is applied at once).
pub async fn cloudflare_check(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let check = crate::services::cloudflare_ranges_service::check(&app).await?;
    Ok(Json(json!({ "check": check })))
}

/// Applies the list Cloudflare offered (more ranges): audited, an action;
/// Traefik restarts when its entrypoints change.
pub async fn cloudflare_apply(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    use crate::services::cloudflare_ranges_service as cf;
    let offered = cf::last_check(app.db()).await?.offered;
    if offered.is_empty() {
        return Err(AppError::BadRequest(
            "Cloudflare offered no new list".into(),
        ));
    }
    let by = Initiator::user(auth.principal(), request);
    cf::apply(&app, offered, Some(&by)).await?;
    Ok(Json(json!({ "applied": cf::applied(app.db()).await? })))
}

fn docker(app: &AppState) -> AppResult<std::sync::Arc<crate::services::docker_service::Docker>> {
    app.service::<crate::services::docker_service::Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))
}

/// The rules: their source, the versions of the last check, exceptions.
pub async fn rules(State(app): State<AppState>) -> AppResult<Json<Value>> {
    use crate::services::waf_rules_service as rules;
    let settings = Settings::load(app.db()).await?;
    let check = rules::last_check(app.db()).await?;
    Ok(Json(json!({
        "source": settings.rules,
        "check": check,
        "outdated": check.outdated(),
        "bundled": rules::bundled(),
        "exceptions": settings.exceptions,
    })))
}

/// Asks CrowdSec's hub which versions are out (its index only).
pub async fn rules_check(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let _rules = crate::services::waf_rules_service::claim()?;
    let check = crate::services::waf_rules_service::check(&app, &docker).await?;
    Ok(Json(
        json!({ "check": check, "outdated": check.outdated() }),
    ))
}

/// Updates the rules (an action: audited; the bundle back if it fails).
pub async fn rules_update(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    use crate::services::waf_rules_service as rules;
    if Settings::load(app.db()).await?.mode == Mode::Off {
        return Err(AppError::BadRequest("the WAF is off".into()));
    }
    let docker = docker(&app)?;
    let claimed = rules::claim()?;
    let by = Initiator::user(auth.principal(), request);
    let txn = rustcms::db::begin_write(app.db()).await?;
    // `latest` from now on (with its audit entry): no reconcile copies the
    // bundle over the update meanwhile.
    let mut settings = Settings::load_for_change(&txn).await?;
    let switching = settings.rules != crate::services::waf_service::RulesSource::Latest;
    settings.rules = crate::services::waf_service::RulesSource::Latest;
    settings.store(&txn).await?;
    audit::record(
        &txn,
        &by.audit,
        AuditEntry::new("waf.rules_update", "panel_settings")
            .key(crate::services::waf_service::SETTINGS_KEY),
    )
    .await?;
    let row = crate::services::action_service::start(
        &txn,
        &by,
        rules::UPDATE_ACTION,
        crate::services::waf_service::CROWDSEC,
        crate::models::action::RUNNING,
    )
    .await?;
    txn.commit().await?;
    let action = row.id;
    tokio::spawn(async move {
        if let Err(err) = rules::update(&app, &docker, row, switching, claimed).await {
            tracing::warn!(%err, "the WAF's rules not updated");
        }
    });
    Ok(Json(json!({ "action": action })))
}

/// Back to the bundled rules (audited; a reconcile restarts CrowdSec).
pub async fn rules_bundled(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let claimed = crate::services::waf_rules_service::claim()?;
    let by = Initiator::user(auth.principal(), request);
    let txn = rustcms::db::begin_write(app.db()).await?;
    let mut settings = Settings::load_for_change(&txn).await?;
    settings.rules = crate::services::waf_service::RulesSource::Bundled;
    settings.store(&txn).await?;
    audit::record(
        &txn,
        &by.audit,
        AuditEntry::new("waf.rules_bundled", "panel_settings")
            .key(crate::services::waf_service::SETTINGS_KEY),
    )
    .await?;
    let (row, status) = announce(&txn, &by).await?;
    txn.commit().await?;
    // The rules stay claimed until the bundle is copied and CrowdSec runs
    // with it (no update meanwhile).
    tokio::spawn(async move {
        let _rules = claimed;
        if let Err(err) = waf_docker_service::reconcile_with(&app, false, Some(row)).await {
            tracing::warn!(%err, "the WAF not reconciled");
        }
    });
    Ok(Json(json!({ "status": status })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExceptionBody {
    pub rule: String,
    pub host: String,
}

/// Spares a domain a rule (audited; a reconcile restarts CrowdSec).
pub async fn exception_add(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<ExceptionBody>,
) -> AppResult<Json<Value>> {
    use crate::services::waf_service::{EXCEPTIONS_MAX, Exception};
    let exception = Exception {
        rule: body.rule.trim().to_owned(),
        host: body.host.trim().to_ascii_lowercase(),
        since: Some(chrono::Utc::now()),
    };
    if let Some((field, problem)) = exception.problem() {
        return Err(ValidationErrors::field(field, problem).into());
    }
    change_exceptions(
        &app,
        auth,
        request,
        "waf.exception_add",
        &exception,
        |list| {
            if list
                .iter()
                .any(|e| e.rule == exception.rule && e.host == exception.host)
            {
                return Ok(false);
            }
            if list.len() >= EXCEPTIONS_MAX {
                return Err(ValidationErrors::field(
                    "rule",
                    format!("at most {EXCEPTIONS_MAX} exceptions"),
                )
                .into());
            }
            list.push(exception.clone());
            Ok(true)
        },
    )
    .await
}

/// A domain checked by a rule again.
pub async fn exception_remove(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<ExceptionBody>,
) -> AppResult<Json<Value>> {
    let exception = crate::services::waf_service::Exception {
        rule: body.rule.trim().to_owned(),
        host: body.host.trim().to_ascii_lowercase(),
        since: None,
    };
    change_exceptions(
        &app,
        auth,
        request,
        "waf.exception_remove",
        &exception,
        |list| {
            let before = list.len();
            list.retain(|e| !(e.rule == exception.rule && e.host == exception.host));
            Ok(list.len() != before)
        },
    )
    .await
}

async fn change_exceptions(
    app: &AppState,
    auth: Auth,
    request: RequestMeta,
    event: &str,
    exception: &crate::services::waf_service::Exception,
    change: impl FnOnce(&mut Vec<crate::services::waf_service::Exception>) -> AppResult<bool>,
) -> AppResult<Json<Value>> {
    let by = Initiator::user(auth.principal(), request);
    let txn = rustcms::db::begin_write(app.db()).await?;
    let mut settings = Settings::load_for_change(&txn).await?;
    if !change(&mut settings.exceptions)? {
        return Ok(Json(
            json!({ "exceptions": settings.exceptions, "changed": false }),
        ));
    }
    settings.store(&txn).await?;
    audit::record(
        &txn,
        &by.audit,
        AuditEntry::new(event.to_owned(), "panel_settings")
            .key(crate::services::waf_service::SETTINGS_KEY)
            .meta(json!({ "rule": exception.rule, "host": exception.host })),
    )
    .await?;
    let (row, status) = announce(&txn, &by).await?;
    txn.commit().await?;
    waf_docker_service::reconcile_in_background_with(app, false, Some(row));
    Ok(Json(
        json!({ "exceptions": settings.exceptions, "changed": true, "status": status }),
    ))
}

/// CrowdSec's output as it comes: the last `tail` lines first.
pub async fn logs(
    live: rustcms::auth::LiveSession,
    State(app): State<AppState>,
    Path(tail): Path<u32>,
) -> AppResult<rustcms::auth::EventStream> {
    let docker = docker(&app)?;
    let query = crate::services::docker_service::LogQuery {
        follow: true,
        tail: Some(tail.min(5_000)),
        since: None,
        timestamps: false,
    };
    let lines = docker
        .engine()
        .service_logs(crate::services::waf_service::CROWDSEC, query);
    Ok(live.stream(crate::services::live_service::log_events(lines)))
}

/// One of the WAF's actions (a start, a rules update, a restart, Cloudflare's
/// ranges applied): its log, for the WAF page's administrators (no
/// `docker.view` needed). Any other action is not found here.
pub async fn action(State(app): State<AppState>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    use sea_orm::EntityTrait;
    let row = crate::models::action::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .filter(|row| row.kind.starts_with("waf.") || row.kind.starts_with("cloudflare."))
        .ok_or(AppError::NotFound)?;
    Ok(Json(json!({ "action": row })))
}

/// CrowdSec started again (an action, audited): its rules loaded anew.
pub async fn restart(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    crate::services::action_service::run(
        app.db(),
        &by,
        "waf.restart",
        crate::services::waf_service::CROWDSEC,
        waf_docker_service::restart_crowdsec(&docker),
    )
    .await?;
    Ok(Json(json!({ "restarted": true })))
}

/// The canary's backend: a constant (Traefik routes the canary host here).
pub async fn canary() -> &'static str {
    waf_docker_service::CANARY_ANSWER
}
