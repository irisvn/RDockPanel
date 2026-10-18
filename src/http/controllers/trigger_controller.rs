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

/// Whether `signature` (`sha256=<hex>`, GitHub's `X-Hub-Signature-256`)
/// signs `body` with `secret` (compared in constant time).
pub fn signed(secret: &str, body: &[u8], signature: &str) -> bool {
    use hmac::{KeyInit, Mac};
    let Some(hex) = signature.strip_prefix("sha256=") else {
        return false;
    };
    let mut mac =
        hmac::Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key");
    mac.update(body);
    let expected: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    expected.len() == hex.len()
        && expected
            .bytes()
            .zip(hex.bytes())
            .fold(true, |ok, (a, b)| ok & (a == b))
}

/// GitHub's webhook (`POST /api/v1/github/{id}`, no session: the signature
/// is the proof): a push to the service's branch deploys it (auto deploy);
/// a `ping` answers; anything else is ignored.
pub async fn github(
    State(app): State<AppState>,
    request: RequestMeta,
    Path(id): Path<String>,
    headers: rustcms::axum::http::HeaderMap,
    body: rustcms::axum::body::Bytes,
) -> AppResult<(StatusCode, Json<Value>)> {
    let id: i64 = id.parse().map_err(|_| AppError::NotFound)?;
    let row = service::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let secret = row
        .webhook_secret
        .as_ref()
        .and_then(|s| {
            s.open(app.keyring(), &service::webhook_secret_context())
                .ok()
        })
        .ok_or(AppError::NotFound)?;
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !signed(&secret, &body, signature) {
        return Err(AppError::NotFound);
    }
    let event = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if event == "ping" {
        return Ok((StatusCode::OK, Json(json!({ "pong": true }))));
    }
    if event != "push" {
        return Ok((StatusCode::OK, Json(json!({ "ignored": event }))));
    }
    let payload: Value = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("the payload is not JSON".into()))?;
    let branch = row
        .git_branch
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .unwrap_or("main");
    let pushed = payload["ref"].as_str().unwrap_or_default();
    if pushed != format!("refs/heads/{branch}") && pushed != format!("refs/tags/{branch}") {
        return Ok((
            StatusCode::OK,
            Json(json!({ "ignored": format!("a push to {pushed}, not {branch}") })),
        ));
    }
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = service::Entity::find_by_id(id)
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    let deployment =
        deployment_service::request(&txn, &row, Trigger::Deploy, None, None, app.keyring()).await?;
    queue::dispatch(
        &txn,
        &RunDeployment {
            deployment_id: deployment.id,
        },
    )
    .await?;
    let entry = AuditEntry::new("services.deploy.github", "services")
        .key(id.to_string())
        .meta(json!({ "deployment": deployment.id, "commit": payload["after"] }));
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
