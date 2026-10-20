//! Settings → Branding (`/admin/api/x/apps/branding`): the panel's name,
//! primary color, logos and favicon. The panel's administrators' only
//! (`system.manage`); each change audited (never the images' bytes).

use base64::Engine;
use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::{AppResult, Json, ValidationErrors};
use serde::Deserialize;
use serde_json::Value;

use crate::services::branding_service;

/// Records the change's audit entry in its transaction `txn` (both or
/// neither), commits, then shows it on this instance's next request.
async fn finish(
    app: &AppState,
    auth: &Auth,
    request: RequestMeta,
    meta: Value,
    txn: sea_orm::DatabaseTransaction,
) -> AppResult<Json<Value>> {
    let entry = AuditEntry::new("branding.update", "panel_settings")
        .key("branding")
        .meta(meta);
    let context = AuditContext::new(AuditActor::user(auth.principal()), request);
    audit::record(&txn, &context, entry).await?;
    txn.commit().await?;
    branding_service::invalidate(app);
    Ok(Json(branding_service::current(app.db()).await?))
}

/// `GET /branding`.
pub async fn index(State(app): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(branding_service::current(app.db()).await?))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Texts {
    pub name: Option<String>,
    pub primary: Option<String>,
}

/// `PUT /branding`: the name and the primary color (empty: the static ones).
pub async fn update(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Texts>,
) -> AppResult<Json<Value>> {
    let txn = rustcms::db::begin_write(app.db()).await?;
    branding_service::save(&txn, body.name.as_deref(), body.primary.as_deref()).await?;
    let meta = json!({ "name": body.name, "primary": body.primary });
    finish(&app, &auth, request, meta, txn).await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Image {
    pub mime: String,
    /// The file, base64.
    pub data: String,
}

/// `PUT /branding/images/{kind}`.
pub async fn upload(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(kind): Path<String>,
    Json(body): Json<Image>,
) -> AppResult<Json<Value>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.data.trim())
        .map_err(|_| ValidationErrors::field("data", "is not base64"))?;
    let meta = json!({ "image": kind, "mime": body.mime, "size": bytes.len() });
    let txn = rustcms::db::begin_write(app.db()).await?;
    branding_service::set_image(&txn, &kind, &body.mime, bytes).await?;
    finish(&app, &auth, request, meta, txn).await
}

/// `DELETE /branding/images/{kind}`.
pub async fn remove(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(kind): Path<String>,
) -> AppResult<Json<Value>> {
    let txn = rustcms::db::begin_write(app.db()).await?;
    branding_service::remove_image(&txn, &kind).await?;
    let meta = json!({ "image": kind, "removed": true });
    finish(&app, &auth, request, meta, txn).await
}
