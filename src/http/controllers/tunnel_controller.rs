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

// Many tunnels (`/tunnels`), one per Cloudflare token, named.

/// Runs `change` whole (one change at a time), even when the request is cut.
async fn changing<F>(change: F) -> AppResult<()>
where
    F: std::future::Future<Output = AppResult<()>> + Send + 'static,
{
    tokio::spawn(async move {
        let _changing = tunnel_service::exclusive().await;
        change.await
    })
    .await
    .map_err(AppError::internal)?
}

async fn audited_tunnel(
    app: &AppState,
    auth: &Auth,
    request: RequestMeta,
    action: &str,
    name: &str,
) {
    let entry = AuditEntry::new(action, "tunnels").key(name);
    audit::record_detached(
        app.db(),
        &AuditContext::new(AuditActor::user(auth.principal()), request),
        entry,
    )
    .await;
}

/// `GET /tunnels`: every tunnel with its state (never a token).
pub async fn index(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    Ok(Json(
        serde_json::json!({ "tunnels": tunnel_service::list(&app, &docker).await? }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Create {
    #[serde(default)]
    pub name: String,
    pub token: Option<String>,
}

/// `POST /tunnels`: a new tunnel, on.
pub async fn create(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Create>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let name = body.name.trim().to_owned();
    let (state, engine, named) = (app.clone(), docker.clone(), name.clone());
    changing(async move {
        if crate::models::tunnel::named(state.db(), &named)
            .await?
            .is_some()
        {
            return Err(ValidationErrors::field("name", "is already a tunnel").into());
        }
        tunnel_service::enable(&state, &engine, &named, body.token.as_deref()).await
    })
    .await?;
    audited_tunnel(&app, &auth, request, "tunnel.create", &name).await;
    Ok(Json(tunnel_service::status(&app, &docker, &name).await?))
}

/// `POST /tunnels/{name}/enable`: on, with a new token when given.
pub async fn enable_one(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(name): Path<String>,
    Json(body): Json<Enable>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    // Unknown: not found (never made here).
    tunnel_service::status(&app, &docker, &name).await?;
    let (state, engine, named) = (app.clone(), docker.clone(), name.clone());
    changing(async move {
        tunnel_service::enable(&state, &engine, &named, body.token.as_deref()).await
    })
    .await?;
    audited_tunnel(&app, &auth, request, "tunnel.enable", &name).await;
    Ok(Json(tunnel_service::status(&app, &docker, &name).await?))
}

/// `POST /tunnels/{name}/disable`: off, its token kept.
pub async fn disable_one(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(name): Path<String>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let (state, engine, named) = (app.clone(), docker.clone(), name.clone());
    changing(async move { tunnel_service::disable(&state, &engine, &named).await }).await?;
    audited_tunnel(&app, &auth, request, "tunnel.disable", &name).await;
    Ok(Json(tunnel_service::status(&app, &docker, &name).await?))
}

/// `DELETE /tunnels/{name}`: stopped and forgotten (its token too).
pub async fn destroy(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(name): Path<String>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let (state, engine, named) = (app.clone(), docker.clone(), name.clone());
    changing(async move { tunnel_service::remove(&state, &engine, &named).await }).await?;
    audited_tunnel(&app, &auth, request, "tunnel.remove", &name).await;
    Ok(Json(serde_json::json!({ "removed": name })))
}
