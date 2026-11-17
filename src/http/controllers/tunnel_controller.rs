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

/// Runs `change` whole (one change at a time), then records `entry`, even
/// when the request is cut (a timeout, a closed page): the change and its
/// audit entry run in their own task.
async fn changing<F, Fut>(
    app: &AppState,
    auth: &Auth,
    request: RequestMeta,
    entry: AuditEntry,
    change: F,
) -> AppResult<std::sync::Arc<Docker>>
where
    F: FnOnce(AppState, std::sync::Arc<Docker>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = AppResult<()>> + Send,
{
    let docker = docker(app)?;
    let (state, engine) = (app.clone(), docker.clone());
    let context = AuditContext::new(AuditActor::user(auth.principal()), request);
    tokio::spawn(async move {
        let _changing = tunnel_service::exclusive().await;
        // Written with the change's own write; one that changed only Docker
        // is recorded after it, as it can be.
        let (done, left) =
            tunnel_service::audited(context, entry, change(state.clone(), engine)).await;
        done?;
        if let Some((context, entry)) = left {
            audit::record_detached(state.db(), &context, entry).await;
        }
        Ok::<_, AppError>(())
    })
    .await
    .map_err(AppError::internal)??;
    Ok(docker)
}

fn default_entry(action: &str) -> AuditEntry {
    AuditEntry::new(action, "panel_settings").key("tunnel")
}

fn tunnel_entry(action: &str, name: &str) -> AuditEntry {
    AuditEntry::new(action, "tunnels").key(name)
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
    let docker = changing(
        &app,
        &auth,
        request,
        default_entry("tunnel.enable"),
        move |state, docker| async move {
            tunnel_service::enable(
                &state,
                &docker,
                tunnel_service::DEFAULT,
                body.token.as_deref(),
            )
            .await
        },
    )
    .await?;
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
    let docker = changing(
        &app,
        &auth,
        request,
        default_entry("tunnel.disable"),
        |state, docker| async move {
            tunnel_service::disable(&state, &docker, tunnel_service::DEFAULT).await
        },
    )
    .await?;
    Ok(Json(
        tunnel_service::status(&app, &docker, tunnel_service::DEFAULT).await?,
    ))
}

// Many tunnels (`/tunnels`), one per Cloudflare token, named.

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
    let name = body.name.trim().to_owned();
    let named = name.clone();
    let docker = changing(
        &app,
        &auth,
        request,
        tunnel_entry("tunnel.create", &name),
        move |state, docker| async move {
            if crate::models::tunnel::named(state.db(), &named)
                .await?
                .is_some()
            {
                return Err(ValidationErrors::field("name", "is already a tunnel").into());
            }
            tunnel_service::enable(&state, &docker, &named, body.token.as_deref()).await
        },
    )
    .await?;
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
    let named = name.clone();
    let docker = changing(
        &app,
        &auth,
        request,
        tunnel_entry("tunnel.enable", &name),
        move |state, docker| async move {
            // Unknown (or removed meanwhile): not found, never made here.
            tunnel_service::known(&state, &named).await?;
            tunnel_service::enable(&state, &docker, &named, body.token.as_deref()).await
        },
    )
    .await?;
    Ok(Json(tunnel_service::status(&app, &docker, &name).await?))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replace {
    #[serde(default)]
    pub token: String,
}

/// `PATCH /tunnels/{name}`: a new token, the tunnel on or off as it was.
pub async fn update(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(name): Path<String>,
    Json(body): Json<Replace>,
) -> AppResult<Json<Value>> {
    let named = name.clone();
    let docker = changing(
        &app,
        &auth,
        request,
        tunnel_entry("tunnel.update", &name),
        move |state, docker| async move {
            tunnel_service::replace_token(&state, &docker, &named, &body.token).await
        },
    )
    .await?;
    Ok(Json(tunnel_service::status(&app, &docker, &name).await?))
}

/// `POST /tunnels/{name}/disable`: off, its token kept.
pub async fn disable_one(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(name): Path<String>,
) -> AppResult<Json<Value>> {
    let named = name.clone();
    let docker = changing(
        &app,
        &auth,
        request,
        tunnel_entry("tunnel.disable", &name),
        move |state, docker| async move { tunnel_service::disable(&state, &docker, &named).await },
    )
    .await?;
    Ok(Json(tunnel_service::status(&app, &docker, &name).await?))
}

/// `DELETE /tunnels/{name}`: stopped and forgotten (its token too).
pub async fn destroy(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(name): Path<String>,
) -> AppResult<Json<Value>> {
    let named = name.clone();
    changing(
        &app,
        &auth,
        request,
        tunnel_entry("tunnel.remove", &name),
        move |state, docker| async move { tunnel_service::remove(&state, &docker, &named).await },
    )
    .await?;
    Ok(Json(serde_json::json!({ "deleted": name })))
}
