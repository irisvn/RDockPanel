//! A service's maintenance page in the panel (`/admin/api/x/apps/…`): its
//! own logo, previews of a page being written, and the panel's shared page
//! (Settings). The page itself is `trigger_controller::maintenance`.
//!
//! A preview is rendered as the page would be, kept a few minutes for its
//! author only, and shown in the panel's frame with the page's sandbox: a
//! user's HTML never runs on the panel's origin.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use sea_orm::EntityTrait;
use serde::Deserialize;
use serde_json::Value;

use crate::http::controllers::apps_controller::{forbid_privileged, read_body, service_for};
use crate::models::{panel_setting, service_logo};
use crate::services::maintenance_service as page;

/// How long a preview is kept, how many in all and by author.
const PREVIEW_TTL: Duration = Duration::from_secs(300);
const PREVIEWS: usize = 20;
const PREVIEWS_EACH: usize = 3;

struct Preview {
    id: String,
    author: String,
    service_id: Option<i64>,
    html: String,
    at: Instant,
}

static PREVIEWS_KEPT: Mutex<Vec<Preview>> = Mutex::new(Vec::new());

fn context(auth: &Auth, request: RequestMeta) -> AuditContext {
    AuditContext::new(AuditActor::user(auth.principal()), request)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Logo {
    /// The file, base64 (PNG, JPEG or WebP).
    pub data: String,
}

/// `PUT /services/{id}/maintenance-logo`: its own logo on its page. Audited.
pub async fn upload_logo(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let row = service_for(&app, &auth, id, "services.update").await?;
    forbid_privileged(&auth, &row)?;
    let body: Logo = read_body(body)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.data.trim())
        .map_err(|_| ValidationErrors::field("data", "is not base64"))?;
    if bytes.len() > page::MAX_LOGO {
        return Err(ValidationErrors::field("data", "is larger than 256 KB").into());
    }
    let mime = page::raster_mime(&bytes)
        .ok_or_else(|| ValidationErrors::field("data", "must be a PNG, JPEG or WebP image"))?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    service_logo::Entity::delete_by_id(row.id)
        .exec(&txn)
        .await?;
    sea_orm::ActiveModelTrait::insert(
        service_logo::ActiveModel {
            service_id: sea_orm::Set(row.id),
            mime: sea_orm::Set(mime.to_owned()),
            data: sea_orm::Set(bytes.clone()),
            updated_at: sea_orm::Set(chrono::Utc::now()),
        },
        &txn,
    )
    .await?;
    let entry = AuditEntry::new("services.maintenance_logo.set", "services")
        .key(row.id.to_string())
        .meta(json!({ "mime": mime, "size": bytes.len() }));
    audit::record(&txn, &context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(json!({ "logo": logo_url(mime, &bytes) })))
}

/// `DELETE /services/{id}/maintenance-logo`: back to the panel's. Audited.
pub async fn remove_logo(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = service_for(&app, &auth, id, "services.update").await?;
    forbid_privileged(&auth, &row)?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    service_logo::Entity::delete_by_id(row.id)
        .exec(&txn)
        .await?;
    let entry =
        AuditEntry::new("services.maintenance_logo.remove", "services").key(row.id.to_string());
    audit::record(&txn, &context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(json!({ "logo": null })))
}

fn logo_url(mime: &str, data: &[u8]) -> String {
    format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(data)
    )
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct PreviewBody {
    /// A service's page (its logo, project, name); none: Settings' shared
    /// page (the panel's administrators').
    pub service_id: Option<i64>,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub eta: Option<chrono::DateTime<chrono::Utc>>,
    pub eta_offset: Option<i16>,
    pub contact: Option<String>,
    /// `default` (the shared page, else the built-in one) or `custom`.
    pub page: Option<String>,
    /// The service's own HTML (`custom`), or the shared page being written.
    pub html: Option<String>,
}

/// `POST /maintenance/preview`: renders a page being written (nothing
/// saved); `{id}` to show at `GET /maintenance/preview/{id}`.
pub async fn create_preview(
    State(app): State<AppState>,
    auth: Auth,
    headers: rustcms::axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let service_id = body.get("service_id").and_then(Value::as_i64);
    let (row, project) = match service_id {
        Some(id) => {
            let row = service_for(&app, &auth, id, "services.view").await?;
            let project = crate::models::project::Entity::find_by_id(row.project_id)
                .one(app.db())
                .await?
                .map(|p| p.name)
                .unwrap_or_default();
            (Some(row), project)
        }
        None if auth
            .principal()
            .can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION) =>
        {
            (None, "project".to_owned())
        }
        None => return Err(AppError::Forbidden),
    };
    let body: PreviewBody = read_body(body)?;
    if body.html.as_ref().is_some_and(|h| h.len() > page::MAX_HTML) {
        return Err(ValidationErrors::field("html", "is larger than 256 KB").into());
    }
    let shared = panel_setting::get(app.db(), page::TEMPLATE_KEY).await?;
    let default_contact = panel_setting::get(app.db(), page::CONTACT_KEY).await?;
    let logo = page::logo_of(app.db(), service_id).await?;
    let contact = body
        .contact
        .clone()
        .filter(|c| !c.trim().is_empty() && page::valid_contact(c))
        .or(default_contact);
    let facts = page::Facts {
        title: body.title.clone().unwrap_or_default(),
        subtitle: body.subtitle.clone().unwrap_or_default(),
        eta: body.eta.map(|at| (at, body.eta_offset.unwrap_or(0))),
        contact,
        logo: logo.0,
        logo_dark: logo.1,
        service: row.as_ref().map_or("web".to_owned(), |r| r.name.clone()),
        project,
        // As a visitor in the author's language would see it.
        lang: page::Lang::pick(
            None,
            headers
                .get(rustcms::axum::http::header::ACCEPT_LANGUAGE)
                .and_then(|h| h.to_str().ok()),
        ),
    };
    // A service's page: its own HTML, else the shared page; Settings': the
    // one being written.
    let template = match (&row, body.page.as_deref()) {
        (Some(_), Some("custom")) => body.html.clone(),
        (Some(_), _) => shared,
        (None, _) => body.html.clone(),
    };
    let html = page::page(template.as_deref(), &facts);
    let id = crate::services::database_service::random_string(24).to_lowercase();
    let author = auth.principal().id.clone();
    let mut kept = PREVIEWS_KEPT.lock().unwrap_or_else(|e| e.into_inner());
    kept.retain(|p| p.at.elapsed() < PREVIEW_TTL);
    // The author's oldest beyond their share, then everyone's oldest.
    while kept.iter().filter(|p| p.author == author).count() >= PREVIEWS_EACH {
        let oldest = kept.iter().position(|p| p.author == author).unwrap();
        kept.remove(oldest);
    }
    while kept.len() >= PREVIEWS {
        kept.remove(0);
    }
    kept.push(Preview {
        id: id.clone(),
        author,
        service_id,
        html,
        at: Instant::now(),
    });
    Ok(Json(json!({ "id": id })))
}

/// `GET /maintenance/preview/{id}`: a preview, to its author only (who may
/// still see its service), with the page's sandbox (framed by the panel).
pub async fn show_preview(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> AppResult<rustcms::axum::response::Response> {
    use rustcms::axum::http::{HeaderValue, header};
    use rustcms::axum::response::IntoResponse;
    let (service_id, html) = {
        let kept = PREVIEWS_KEPT.lock().unwrap_or_else(|e| e.into_inner());
        let found = kept
            .iter()
            .find(|p| p.id == id && p.author == auth.principal().id && p.at.elapsed() < PREVIEW_TTL)
            .ok_or(AppError::NotFound)?;
        (found.service_id, found.html.clone())
    };
    match service_id {
        Some(service) => {
            service_for(&app, &auth, service, "services.view").await?;
        }
        None if auth
            .principal()
            .can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION) => {}
        None => return Err(AppError::NotFound),
    }
    let mut response = html.into_response();
    let set = response.headers_mut();
    set.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    set.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    set.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(page::PREVIEW_CSP),
    );
    set.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    set.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );
    Ok(response)
}

/// `GET /settings/maintenance`: the panel's shared page and default contact,
/// with the built-in page's HTML (to start from) and the placeholders.
pub async fn show_settings(State(app): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(settings(&app).await?))
}

async fn settings(app: &AppState) -> AppResult<Value> {
    Ok(json!({
        "template": panel_setting::get(app.db(), page::TEMPLATE_KEY).await?.unwrap_or_default(),
        "contact": panel_setting::get(app.db(), page::CONTACT_KEY).await?.unwrap_or_default(),
        "starter": page::starter(),
        "placeholders": page::PLACEHOLDERS,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub template: String,
    pub contact: String,
}

/// `PUT /settings/maintenance`: the shared page (empty: the built-in one)
/// and the default contact. Audited (not the HTML).
pub async fn update_settings(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Settings>,
) -> AppResult<Json<Value>> {
    let mut errors = ValidationErrors::new();
    if body.template.len() > page::MAX_HTML {
        errors.add("template", "is larger than 256 KB");
    }
    let contact = body.contact.trim();
    if !contact.is_empty() && !page::valid_contact(contact) {
        errors.add("contact", "must be an email or an https link");
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }
    let txn = rustcms::db::begin_write(app.db()).await?;
    panel_setting::set(&txn, page::TEMPLATE_KEY, body.template.trim()).await?;
    panel_setting::set(&txn, page::CONTACT_KEY, contact).await?;
    let entry = AuditEntry::new("maintenance.settings", "panel_settings")
        .key("maintenance")
        .meta(json!({ "template_size": body.template.trim().len(), "contact": contact }));
    audit::record(&txn, &context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(settings(&app).await?))
}
