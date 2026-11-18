//! Settings → Security (the WAF): its settings, its state (the canary's),
//! repair, and CrowdSec's latest alerts — the panel's administrators only
//! (`system.manage`, routes/admin.rs). The canary's backend is public: it
//! answers a constant, nothing else.

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::services::docker_service::Docker;
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
    let settings = Settings {
        mode: body.mode,
        level: body.level,
        allowlist,
        sharing: body.sharing,
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
    let txn = rustcms::db::begin_write(app.db()).await?;
    let before = Settings::load(&txn).await?;
    settings.store(&txn).await?;
    let entry = AuditEntry::new("waf.settings", "panel_settings")
        .key(crate::services::waf_service::SETTINGS_KEY)
        .meta(json!({ "before": before, "after": settings }));
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(auth.principal()), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    waf_docker_service::reconcile_in_background(&app);
    Ok(Json(
        json!({ "settings": settings, "status": { "state": if settings.mode == Mode::Off { "off" } else { "starting" } } }),
    ))
}

/// Brings everything to the settings again (the plugin copied again,
/// CrowdSec and Traefik checked): after an "unavailable".
pub async fn repair(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let txn = rustcms::db::begin_write(app.db()).await?;
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(auth.principal()), request),
        AuditEntry::new("waf.repair", "panel_settings")
            .key(crate::services::waf_service::SETTINGS_KEY),
    )
    .await?;
    txn.commit().await?;
    waf_docker_service::reconcile_in_background(&app);
    Ok(Json(json!({ "status": { "state": "starting" } })))
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
    let txn = rustcms::db::begin_write(app.db()).await?;
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(auth.principal()), request),
        AuditEntry::new("waf.rotate_key", "panel_settings")
            .key(crate::services::waf_service::SETTINGS_KEY),
    )
    .await?;
    txn.commit().await?;
    waf_docker_service::reconcile_in_background_with(&app, true);
    Ok(Json(json!({ "status": { "state": "starting" } })))
}

/// How many of CrowdSec's latest alerts the page shows.
const ALERTS: usize = 100;

/// CrowdSec's latest alerts (never request bodies).
pub async fn alerts(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))?;
    let settings = Settings::load(app.db()).await?;
    // CrowdSec not answering is the page's to say (never "no alert").
    let alerts = if settings.mode == Mode::Off {
        vec![]
    } else {
        waf_docker_service::alerts(&docker, ALERTS).await?
    };
    Ok(Json(json!({ "alerts": alerts, "limit": ALERTS })))
}

/// The canary's backend: a constant (Traefik routes the canary host here).
pub async fn canary() -> &'static str {
    waf_docker_service::CANARY_ANSWER
}
