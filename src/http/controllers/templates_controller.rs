//! EasyPanel templates (`routes/admin.rs`, under `/admin/api/x/apps`): the
//! catalog, a template's form, and creating a template's services in a
//! project (or the same `{services}` pasted: "Create from Schema"),
//! deployed at once when asked (databases first).

use std::sync::Arc;

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::queue;
use rustcms::{AppError, AppResult, Json};
use sea_orm::EntityTrait;
use serde::Deserialize;
use serde_json::Value;

use crate::jobs::run_deployment::RunDeployment;
use crate::models::project;
use crate::services::creation_service;
use crate::services::deploy_rules_service::config;
use crate::services::deployment_service::{self, Trigger};
use crate::services::template_engine;
use crate::services::template_service::{self, Plan, Templates};

fn templates(app: &AppState) -> AppResult<Arc<Templates>> {
    app.service::<Templates>()
        .ok_or_else(|| AppError::internal("Templates is not bound"))
}

fn unavailable(message: String) -> AppError {
    AppError::BadRequest(message)
}

/// The catalog (fetched from the templates' repository, kept a few hours).
pub async fn list(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let list = templates(&app)?.list().await.map_err(unavailable)?;
    Ok(Json(json!({ "templates": list })))
}

/// Fetches the catalog and templates again.
pub async fn refresh(State(app): State<AppState>) -> AppResult<Json<Value>> {
    templates(&app)?.refresh();
    Ok(Json(json!({ "ok": true })))
}

/// A template's logo (through the panel: its CSP loads images from itself).
pub async fn logo(
    State(app): State<AppState>,
    Path(slug): Path<String>,
) -> AppResult<rustcms::axum::response::Response> {
    use rustcms::axum::http::header;
    use rustcms::axum::response::IntoResponse;
    let (kind, bytes) = templates(&app)?
        .logo(&slug)
        .await
        .map_err(|_| AppError::NotFound)?;
    // An SVG is shown as an image only (no scripts run in an <img>), and
    // never as a document of the panel's origin.
    Ok((
        [
            (header::CONTENT_TYPE, kind),
            (header::CACHE_CONTROL, "private, max-age=21600"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; sandbox",
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}

/// A template's page: name, description, instructions, its form's schema.
pub async fn show(State(app): State<AppState>, Path(slug): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(
        templates(&app)?.page(&slug).await.map_err(unavailable)?,
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UseTemplate {
    /// The form's values.
    #[serde(default)]
    pub input: Value,
    /// Deploy the services once created.
    #[serde(default)]
    pub deploy: bool,
}

/// Runs template `slug` with `input` and creates its services in project `id`.
pub async fn create(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path((id, slug)): Path<(i64, String)>,
    Json(body): Json<UseTemplate>,
) -> AppResult<Json<Value>> {
    let source = templates(&app)?;
    let (meta, code) = source.template(&slug).await.map_err(unavailable)?;
    let input = with_defaults(&meta, body.input);
    // In a child process with its limits (never in the panel's).
    let output = template_engine::run_isolated(&source.runner, &code, &meta, &input)
        .await
        .map_err(unavailable)?;
    apply(&app, &auth, request, id, &output, body.deploy, Some(&slug)).await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FromSchema {
    /// `{services: [...]}`, as a template's generator answers.
    pub schema: Value,
    #[serde(default)]
    pub deploy: bool,
}

/// "Create from Schema": a generator's output, pasted.
pub async fn from_schema(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<FromSchema>,
) -> AppResult<Json<Value>> {
    apply(&app, &auth, request, id, &body.schema, body.deploy, None).await
}

/// The form's defaults (`schema.properties.*.default`) under what was sent.
fn with_defaults(meta: &Value, input: Value) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(properties) = meta
        .pointer("/schema/properties")
        .and_then(Value::as_object)
    {
        for (name, property) in properties {
            if let Some(default) = property.get("default") {
                out.insert(name.clone(), default.clone());
            }
        }
    }
    if let Value::Object(sent) = input {
        out.extend(sent);
    }
    Value::Object(out)
}

/// Creates the planned services (one transaction: all or none), then asks
/// for their deployments when `deploy`.
async fn apply(
    app: &AppState,
    auth: &Auth,
    request: RequestMeta,
    project_id: i64,
    output: &Value,
    deploy: bool,
    slug: Option<&str>,
) -> AppResult<Json<Value>> {
    let principal = auth.principal();
    if !principal.can("projects.view") {
        return Err(AppError::NotFound);
    }
    if deploy && !principal.can("services.deploy") {
        return Err(AppError::Forbidden);
    }
    let txn = rustcms::db::begin_write(app.db()).await?;
    let project = project::Entity::find_by_id(project_id)
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    let host = (!config().service_domain.trim().is_empty()).then(|| config().service_domain.trim());
    let Plan {
        services,
        warnings,
        blocking,
    } = template_service::plan(output, &project.name, host).map_err(unavailable)?;
    if deploy && !blocking.is_empty() {
        return Err(unavailable(format!(
            "{} Create it without deploying, and protect it before deploying.",
            blocking.join("; ")
        )));
    }
    // The related records each need their own resource's permission.
    for (wanted, permission) in [
        (
            services.iter().any(|s| !s.domains.is_empty()),
            "service_domains.create",
        ),
        (
            services.iter().any(|s| !s.ports.is_empty()),
            "service_ports.create",
        ),
        (
            services.iter().any(|s| !s.mounts.is_empty()),
            "service_mounts.create",
        ),
    ] {
        if wanted && !principal.can(permission) {
            return Err(AppError::Forbidden);
        }
    }
    let mut created = vec![];
    for new in services {
        let row = creation_service::create(&txn, app.keyring(), &project, new).await?;
        created.push(row);
    }
    let user = principal.id.parse().ok();
    if deploy {
        for row in &created {
            let deployment =
                deployment_service::request(&txn, row, Trigger::Deploy, None, user, app.keyring())
                    .await?;
            queue::dispatch(
                &txn,
                &RunDeployment {
                    deployment_id: deployment.id,
                },
            )
            .await?;
        }
    }
    let names: Vec<&str> = created.iter().map(|s| s.name.as_str()).collect();
    let entry = AuditEntry::new("services.template", "services")
        .key(project.name.clone())
        .meta(json!({ "template": slug, "services": names, "deploy": deploy }));
    let context = AuditContext::new(AuditActor::user(principal), request);
    audit::record(&txn, &context, entry).await?;
    txn.commit().await?;
    Ok(Json(json!({
        "project": project.name,
        "services": created
            .iter()
            .map(|s| json!({ "id": s.id, "name": s.name, "kind": s.kind }))
            .collect::<Vec<_>>(),
        "warnings": warnings,
    })))
}
