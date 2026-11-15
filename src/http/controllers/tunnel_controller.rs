//! Settings → Cloudflare Tunnel (`/admin/api/x/apps/tunnel`): the `default`
//! tunnel's state, on (with a token) and off. The panel's administrators' only
//! (`system.manage`): it changes Traefik and runs cloudflared. Audited
//! (never the token).

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use serde::Deserialize;
use serde_json::Value;

use crate::services::docker_service::Docker;
use crate::services::tunnel_service;

fn docker(app: &AppState) -> AppResult<std::sync::Arc<Docker>> {
    app.service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))
}

async fn audited(app: &AppState, auth: &Auth, request: RequestMeta, action: &str) {
    let entry = AuditEntry::new(action, "panel_settings").key("tunnel");
    audit::record_detached(
        app.db(),
        &AuditContext::new(AuditActor::user(auth.principal()), request),
        entry,
    )
    .await;
}

/// `GET /tunnel`.
pub async fn status(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    Ok(Json(
        tunnel_service::status(&app, &docker, tunnel_service::DEFAULT).await?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enable {
    /// A new token; none: the one kept.
    pub token: Option<String>,
}

/// `POST /tunnel/enable`.
pub async fn enable(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Enable>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    // Run whole even when the request is cut (a timeout, a closed page).
    let (state, engine) = (app.clone(), docker.clone());
    tokio::spawn(async move {
        let _changing = tunnel_service::exclusive().await;
        tunnel_service::enable(
            &state,
            &engine,
            tunnel_service::DEFAULT,
            body.token.as_deref(),
        )
        .await
    })
    .await
    .map_err(AppError::internal)??;
    audited(&app, &auth, request, "tunnel.enable").await;
    Ok(Json(
        tunnel_service::status(&app, &docker, tunnel_service::DEFAULT).await?,
    ))
}

/// `POST /tunnel/disable`.
pub async fn disable(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let (state, engine) = (app.clone(), docker.clone());
    tokio::spawn(async move {
        let _changing = tunnel_service::exclusive().await;
        tunnel_service::disable(&state, &engine, tunnel_service::DEFAULT).await
    })
    .await
    .map_err(AppError::internal)??;
    audited(&app, &auth, request, "tunnel.disable").await;
    Ok(Json(
        tunnel_service::status(&app, &docker, tunnel_service::DEFAULT).await?,
    ))
}
