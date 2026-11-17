//! Docker → Nodes: the Swarm's servers (`docker.view`), adding one (the join
//! command: an audited read of a secret), pausing, draining, activating and
//! removing one (`system.manage`, recorded as actions). The rules are
//! `node_service`'s.

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::axum::http::header;
use rustcms::axum::response::{IntoResponse, Response};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::policies::permissions;
use crate::services::action_service::{self, Initiator};
use crate::services::docker_service::Docker;
use crate::services::node_service::{self, Availability, Refused};

fn docker(app: &AppState) -> AppResult<std::sync::Arc<Docker>> {
    app.service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))
}

fn refused(err: Refused) -> AppError {
    match err {
        Refused::Gone => AppError::NotFound,
        Refused::Rule(message) => AppError::Conflict {
            code: "node_refused",
            reason: "node",
            message,
        },
        Refused::Docker(why) if why.contains("out of sequence") => AppError::Conflict {
            code: "node_changed",
            reason: "node",
            message: "The node changed meanwhile: look again",
        },
        Refused::Docker(why) => AppError::BadRequest(format!("Docker: {why}")),
    }
}

/// The nodes (whitelisted fields; the panel's own tasks counted for its
/// administrators only).
pub async fn index(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    let system = auth.principal().can(permissions::SYSTEM_MANAGE);
    let nodes = node_service::list(&*docker(&app)?, system)
        .await
        .map_err(refused)?;
    Ok(Json(json!({ "nodes": nodes, "can_manage": system })))
}

/// The command a new server joins with: its read is audited first (no
/// audit, no token), never cached. A Swarm others cannot reach (a Mac's,
/// on 127.0.0.1) answers why instead.
pub async fn join_command(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Response> {
    let docker = docker(&app)?;
    let address = node_service::join_address(&docker).await.map_err(refused)?;
    let address = match address {
        Ok(address) => address,
        Err(why) => {
            return Ok(Json(json!({ "command": null, "why": why })).into_response());
        }
    };
    let entry = AuditEntry::new("swarm.join_token.read", "docker").key("worker");
    let txn = rustcms::db::begin_write(app.db()).await?;
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(auth.principal()), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    let token = docker
        .engine()
        .worker_join_token()
        .await
        .map_err(|e| AppError::BadRequest(format!("Docker: {e}")))?;
    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "command": format!("docker swarm join --token {token} {address}") })),
    )
        .into_response())
}

/// A new worker token (the old one stops working); it answers no token.
pub async fn rotate_token(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    action_service::run(
        app.db(),
        &by,
        "swarm.rotate_token",
        "worker",
        docker.engine().rotate_worker_token(),
    )
    .await?;
    Ok(Json(json!({ "rotated": true })))
}

/// What draining or losing a node does to what runs there (Docker's live
/// state, and the panel's services placed on it).
pub async fn impact(State(app): State<AppState>, Path(id): Path<String>) -> AppResult<Json<Value>> {
    let impact = node_service::impact(app.db(), &*docker(&app)?, &id)
        .await
        .map_err(refused)?;
    Ok(Json(impact))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilityBody {
    pub availability: String,
}

/// Pause, drain or activate a node. Recorded as an action.
pub async fn update(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<String>,
    Json(body): Json<AvailabilityBody>,
) -> AppResult<Json<Value>> {
    let availability = Availability::parse(&body.availability).ok_or_else(|| {
        rustcms::ValidationErrors::field("availability", "must be active, pause or drain")
    })?;
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    action_service::run(
        app.db(),
        &by,
        &format!("node.{}", availability.as_str()),
        &id,
        async {
            node_service::set_availability(&docker, &id, availability)
                .await
                .map_err(refused)
        },
    )
    .await?;
    Ok(Json(json!({ "availability": availability.as_str() })))
}

/// Remove a worker that has left the Swarm. Recorded as an action.
pub async fn remove(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    action_service::run(app.db(), &by, "node.remove", &id, async {
        node_service::remove(&docker, &id).await.map_err(refused)
    })
    .await?;
    Ok(Json(json!({ "removed": true })))
}
