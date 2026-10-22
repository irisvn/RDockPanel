//! A service's deployment trigger (`POST /api/v1/deploy/{id}?token=…`, no
//! session: the token is the secret): what a CI or a Git host's webhook
//! calls to deploy. Only the token's SHA-256 is stored; the token is in the
//! query, which the request log leaves out (it records the path).

use std::collections::HashMap;

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::queue;
use rustcms::{AppError, AppResult, Json};
use sea_orm::{EntityTrait, QuerySelect};
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
    // Locked: what is checked is what deploys.
    let row = service::Entity::find_by_id(id)
        .lock_exclusive()
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
    // A privileged service is deployed by its administrators only.
    if crate::models::service::privileged(&row) {
        return Err(AppError::Forbidden);
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
    if crate::models::service::privileged(&row) {
        return Err(AppError::Forbidden);
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
    // The branch built: the service's, else the repository's default (as a
    // clone without a branch takes).
    let branch = row
        .git_branch
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            payload["repository"]["default_branch"]
                .as_str()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "main".to_owned());
    let pushed = payload["ref"].as_str().unwrap_or_default();
    let matches = if branch.starts_with("refs/") {
        pushed == branch
    } else {
        pushed == format!("refs/heads/{branch}") || pushed == format!("refs/tags/{branch}")
    };
    if !matches {
        return Ok((
            StatusCode::OK,
            Json(json!({ "ignored": format!("a push to {pushed}, not {branch}") })),
        ));
    }
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = service::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    // Checked again on the locked row (it may have become privileged).
    if crate::models::service::privileged(&row) {
        return Err(AppError::Forbidden);
    }
    let deployment =
        match deployment_service::request(&txn, &row, Trigger::Deploy, None, None, app.keyring())
            .await
        {
            Ok(deployment) => deployment,
            // One under way: deployed again once it ends (pushes coalesce).
            Err(AppError::Conflict {
                code: "deployment_in_progress",
                ..
            }) => {
                drop(txn);
                use sea_orm::{ColumnTrait, QueryFilter};
                service::Entity::update_many()
                    .col_expr(
                        service::Column::RedeployPending,
                        sea_orm::sea_query::Expr::value(true),
                    )
                    .filter(service::Column::Id.eq(id))
                    .exec(app.db())
                    .await?;
                // The one running may have ended meanwhile (its check
                // done): asked for now if so, else kept for its end.
                match deployment_service::redeploy_if_pending(&app, id).await {
                    Ok(()) | Err(AppError::Conflict { .. }) => {}
                    Err(err) => return Err(err),
                }
                return Ok((StatusCode::ACCEPTED, Json(json!({ "pending": true }))));
            }
            Err(err) => return Err(err),
        };
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

/// The maintenance page of a service (`/api/v1/maintenance/<project>_<service>`,
/// where Traefik routes its domains while it is in maintenance), 503: the
/// built-in page, the panel's shared one or its own HTML, in a sandbox
/// (no script, an opaque origin). Only on its own domains (never the
/// panel's, nor an address): else, unknown or not in maintenance, 404.
pub async fn maintenance(
    State(app): State<AppState>,
    Path(name): Path<String>,
    headers: rustcms::axum::http::HeaderMap,
) -> AppResult<rustcms::axum::response::Response> {
    use crate::services::maintenance_service as page;
    use rustcms::axum::http::header;
    use rustcms::axum::response::IntoResponse;
    use sea_orm::{ColumnTrait, QueryFilter};
    let (project, service_name) = name.split_once('_').ok_or(AppError::NotFound)?;
    let project = crate::models::project::Entity::find()
        .filter(crate::models::project::Column::Name.eq(project))
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let row = service::Entity::find()
        .filter(service::Column::ProjectId.eq(project.id))
        .filter(service::Column::Name.eq(service_name))
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    // In maintenance as saved: what Traefik routes (applied when saved).
    if !row.maintenance {
        return Err(AppError::NotFound);
    }
    // Asked for one of its domains (Traefik keeps the visitor's host).
    let domains: Vec<String> = crate::models::service_domain::Entity::find()
        .filter(crate::models::service_domain::Column::ServiceId.eq(row.id))
        .all(app.db())
        .await?
        .into_iter()
        .map(|d| d.host)
        .collect();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    // Never the panel's own host (a wildcard domain may cover it).
    if crate::services::deploy_rules_service::is_panel_host(&page::bare_host(host))
        || !page::serves(host, &domains)
    {
        return Err(AppError::NotFound);
    }
    let contact = crate::models::panel_setting::get(app.db(), page::CONTACT_KEY).await?;
    let logo = page::logo_of(app.db(), Some(row.id)).await?;
    let mut facts = page::Facts::of(&row, &project.name, contact, logo);
    facts.lang = page::Lang::pick(
        None,
        headers
            .get(header::ACCEPT_LANGUAGE)
            .and_then(|h| h.to_str().ok()),
    );
    let template = page::template_of(app.db(), &row).await?;
    let html = page::page(template.as_deref(), &facts);
    let retry = page::retry_after(row.maintenance_eta, chrono::Utc::now()).to_string();
    let mut response = (StatusCode::SERVICE_UNAVAILABLE, html).into_response();
    let set = response.headers_mut();
    for (name, value) in [
        (header::CONTENT_TYPE, "text/html; charset=utf-8".to_owned()),
        (header::CACHE_CONTROL, "no-store".to_owned()),
        (header::RETRY_AFTER, retry),
        (header::CONTENT_SECURITY_POLICY, page::CSP.to_owned()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_owned()),
        (header::REFERRER_POLICY, "no-referrer".to_owned()),
    ] {
        if let Ok(value) = rustcms::axum::http::HeaderValue::from_str(&value) {
            set.insert(name, value);
        }
    }
    Ok(response)
}
