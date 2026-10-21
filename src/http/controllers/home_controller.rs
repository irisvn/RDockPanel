//! The homepage (`/`, its stylesheet and images under `/home/`), public;
//! and its switch in Settings → Panel (`/admin/api/x/apps/home`,
//! `system.manage`, audited).

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::axum::http::{HeaderMap, HeaderName, header};
use rustcms::axum::response::{IntoResponse, Redirect, Response};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use crate::services::home_service::{self, Lang};

/// Its own policy: its stylesheets, images and the screens' viewer only.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// `GET /`: the homepage, or the panel when it is turned off.
pub async fn page(
    State(app): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> AppResult<Response> {
    if !home_service::enabled(app.db()).await? {
        return Ok(Redirect::to("/admin").into_response());
    }
    let accept = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok());
    let lang = Lang::pick(query.get("lang").map(String::as_str), accept);
    let headers: [(HeaderName, &str); 6] = [
        (header::CONTENT_TYPE, "text/html; charset=utf-8"),
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "same-origin"),
        // Turned off in Settings: the next visit already goes to the panel.
        (header::CACHE_CONTROL, "no-cache"),
        (header::VARY, "Accept-Language"),
    ];
    Ok((
        headers,
        home_service::render(lang, home_service::assets_tag()),
    )
        .into_response())
}

/// The assets: named by their contents (`?v=`), kept a day.
fn asset(content_type: &'static str, body: impl IntoResponse) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        body,
    )
        .into_response()
}

/// `GET /home/home.css`.
pub async fn css() -> Response {
    asset("text/css; charset=utf-8", home_service::CSS)
}

/// `GET /home/home.js`: the screens' viewer.
pub async fn script() -> Response {
    asset("text/javascript; charset=utf-8", home_service::SCRIPT)
}

/// `GET /home/favicon.svg`.
pub async fn favicon() -> Response {
    asset("image/svg+xml", home_service::FAVICON)
}

/// `GET /home/brand.css`: the panel's color (changed at any time).
pub async fn brand(State(app): State<AppState>) -> AppResult<Response> {
    let css = home_service::brand_css(app.db()).await?;
    let mut response = asset("text/css; charset=utf-8", css);
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        rustcms::axum::http::HeaderValue::from_static("no-cache"),
    );
    Ok(response)
}

/// `GET /home/img/{name}`.
pub async fn image(Path(name): Path<String>) -> AppResult<Response> {
    let (_, bytes) = home_service::IMAGES
        .iter()
        .find(|(n, _)| *n == name)
        .ok_or(AppError::NotFound)?;
    Ok(asset("image/jpeg", *bytes))
}

/// `GET /home`: whether `/` shows the homepage.
pub async fn show(State(app): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(
        json!({ "enabled": home_service::enabled(app.db()).await? }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Switch {
    pub enabled: bool,
}

/// `PUT /home`: turns the homepage on or off (audited with it).
pub async fn update(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Switch>,
) -> AppResult<Json<Value>> {
    use sea_orm::TransactionTrait;
    let txn = app.db().begin().await?;
    home_service::set_enabled(&txn, body.enabled).await?;
    let entry = AuditEntry::new("home.update", "panel_settings")
        .key("home")
        .meta(json!({ "enabled": body.enabled }));
    let context = AuditContext::new(AuditActor::user(auth.principal()), request);
    audit::record(&txn, &context, entry).await?;
    txn.commit().await?;
    Ok(Json(json!({ "enabled": body.enabled })))
}
