//! Settings → API tokens (`/admin/api/x/apps/tokens`): each user's own
//! tokens for the public API — made with scopes and an expiry, shown once,
//! revoked. Audited (never the token).

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::auth::entities::personal_access_token as token;
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Deserialize;
use serde_json::Value;

use crate::services::api_token_service::{SCOPES, abilities, scopes_of};

/// The longest a token lasts (days).
const MAX_DAYS: u32 = 365;

fn me(auth: &Auth) -> AppResult<i64> {
    auth.principal().id.parse().map_err(|_| AppError::Forbidden)
}

fn shown(row: &token::Model) -> Value {
    let set = serde_json::from_str::<Vec<String>>(&row.abilities)
        .unwrap_or_default()
        .into_iter()
        .collect();
    json!({
        "id": row.id,
        "name": row.name,
        "scopes": scopes_of(&set),
        "created_at": row.created_at,
        "last_used_at": row.last_used_at,
        "expires_at": row.expires_at,
    })
}

/// The caller's tokens (live ones: not revoked), and the scopes to choose.
pub async fn index(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    let user = me(&auth)?;
    let rows = token::Entity::find()
        .filter(token::Column::UserId.eq(user))
        .filter(token::Column::RevokedAt.is_null())
        .order_by_desc(token::Column::Id)
        .all(app.db())
        .await?;
    Ok(Json(json!({
        "tokens": rows.iter().map(shown).collect::<Vec<_>>(),
        "scopes": SCOPES.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        "max_days": MAX_DAYS,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewToken {
    pub name: String,
    pub scopes: Vec<String>,
    pub days: u32,
}

/// Makes a token: its value is answered once (never stored, never again).
pub async fn create(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<NewToken>,
) -> AppResult<rustcms::axum::response::Response> {
    use rustcms::axum::response::IntoResponse;
    let user = me(&auth)?;
    let name = body.name.trim();
    let mut errors = ValidationErrors::new();
    if name.is_empty() || name.chars().count() > 64 {
        errors.add("name", "is 1 to 64 characters");
    }
    let granted = match abilities(&body.scopes) {
        Ok(granted) => granted,
        Err(message) => {
            errors.add("scopes", message);
            vec![]
        }
    };
    if !(1..=MAX_DAYS).contains(&body.days) {
        errors.add("days", format!("is 1 to {MAX_DAYS}"));
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }
    let expires = rustcms::auth::tokens::expiry_in_days(body.days)
        .ok_or_else(|| ValidationErrors::field("days", "is out of range"))?;
    // The token and its entry together: none made unrecorded.
    let txn = rustcms::db::begin_write(app.db()).await?;
    let issued = rustcms::auth::tokens::issue(&txn, user, name, &granted, Some(expires)).await?;
    let entry = AuditEntry::new("api_tokens.create", "api_tokens")
        .key(issued.token.id.to_string())
        .meta(json!({ "name": name, "scopes": body.scopes, "days": body.days }));
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(auth.principal()), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok((
        [(rustcms::axum::http::header::CACHE_CONTROL, "no-store")],
        Json(json!({ "token": issued.plain_text, "info": shown(&issued.token) })),
    )
        .into_response())
}

/// Revokes one of the caller's tokens.
pub async fn revoke(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let user = me(&auth)?;
    let row = token::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .filter(|t| t.user_id == user)
        .ok_or(AppError::NotFound)?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    rustcms::auth::tokens::revoke(&txn, row.id).await?;
    let entry = AuditEntry::new("api_tokens.revoke", "api_tokens")
        .key(row.id.to_string())
        .meta(json!({ "name": row.name }));
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(auth.principal()), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok(Json(json!({ "revoked": true })))
}
