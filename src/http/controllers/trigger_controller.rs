//! A service's deployment trigger (`POST /api/v1/deploy/{id}?token=…`, no
//! session: the token is the secret): what a CI or a Git host's webhook
//! calls to deploy. Only the token's SHA-256 is stored; the token is in the
//! query, which the request log leaves out (it records the path).

use std::collections::HashMap;

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::queue;
use rustcms::{AppError, AppResult, Json};
use sea_orm::EntityTrait;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::jobs::run_deployment::RunDeployment;
use crate::models::service;
use crate::services::deployment_service::{self, Trigger};

/// SHA-256 of a trigger token, as stored.
pub fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Deploys the service the URL names when its token matches.
pub async fn deploy(
    State(app): State<AppState>,
    request: RequestMeta,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<(StatusCode, Json<Value>)> {
    // Unknown, wrong or malformed: the same answer.
    let id: i64 = id.parse().map_err(|_| AppError::NotFound)?;
    let token = query.get("token").ok_or(AppError::NotFound)?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = service::Entity::find_by_id(id)
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    let expected = row.trigger_hash.as_deref().ok_or(AppError::NotFound)?;
    let given = token_hash(token);
    // The same length always: compared whole, never early.
    let equal = given
        .bytes()
        .zip(expected.bytes())
        .fold(given.len() == expected.len(), |ok, (a, b)| ok & (a == b));
    if !equal {
        return Err(AppError::NotFound);
    }
    let deployment =
        deployment_service::request(&txn, &row, Trigger::Deploy, None, None, app.keyring()).await?;
    queue::dispatch(
        &txn,
        &RunDeployment {
            deployment_id: deployment.id,
        },
    )
    .await?;
    let entry = AuditEntry::new("services.deploy.trigger", "services")
        .key(id.to_string())
        .meta(json!({ "deployment": deployment.id }));
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::system(None), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "deployment": deployment.id })),
    ))
}
