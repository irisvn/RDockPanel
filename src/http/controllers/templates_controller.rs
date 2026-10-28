//! EasyPanel templates (`routes/admin.rs`, under `/admin/api/x/apps`): the
//! catalog, a template's form, and creating a template's services in a
//! project (or the same `{services}` pasted: "Create from Schema"),
//! deployed at once when asked (databases first).

use std::sync::Arc;

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::queue;
use rustcms::{AppError, AppResult, Json};
use sea_orm::{EntityTrait, QuerySelect};
use serde::Deserialize;
use serde_json::Value;

use crate::jobs::run_deployment::RunDeployment;
use crate::models::project;
use crate::policies::permissions;
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

/// Who adds services somewhere (a role's, or a project role's): the
/// catalog is theirs.
async fn creator(app: &AppState, auth: &Auth) -> AppResult<()> {
    if crate::services::access_service::admits(app.db(), auth.principal(), "services.create")
        .await?
    {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

/// The catalog (fetched from the templates' repository, kept a few hours).
pub async fn list(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    creator(&app, &auth).await?;
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
    auth: Auth,
    Path(slug): Path<String>,
) -> AppResult<rustcms::axum::response::Response> {
    creator(&app, &auth).await?;
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
pub async fn show(
    State(app): State<AppState>,
    auth: Auth,
    Path(slug): Path<String>,
) -> AppResult<Json<Value>> {
    creator(&app, &auth).await?;
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
    /// Nothing created: what they would take (`capacity`).
    #[serde(default)]
    pub dry_run: bool,
}

/// Runs template `slug` with `input` and creates its services in project `id`.
pub async fn create(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path((id, slug)): Path<(i64, String)>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    // Allowed in that project first: no generator runs for anyone else.
    crate::services::access_service::project(app.db(), auth.principal(), id, "services.create")
        .await?;
    let body: UseTemplate = serde_json::from_value(body)
        .map_err(|e| AppError::BadRequest(format!("the request: {e}")))?;
    let source = templates(&app)?;
    let (meta, code) = source.template(&slug).await.map_err(unavailable)?;
    let input = with_defaults(&meta, body.input);
    // In a child process with its limits (never in the panel's).
    let output = template_engine::run_isolated(&source.runner, &code, &meta, &input)
        .await
        .map_err(unavailable)?;
    apply(
        &app,
        &auth,
        request,
        id,
        &output,
        (body.deploy, body.dry_run),
        Some(&slug),
    )
    .await
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FromSchema {
    /// `{services: [...]}`, as a template's generator answers.
    pub schema: Value,
    #[serde(default)]
    pub deploy: bool,
    #[serde(default)]
    pub dry_run: bool,
}

/// "Create from Schema": a generator's output, pasted.
pub async fn from_schema(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    crate::services::access_service::project(app.db(), auth.principal(), id, "services.create")
        .await?;
    let body: FromSchema = serde_json::from_value(body)
        .map_err(|e| AppError::BadRequest(format!("the request: {e}")))?;
    apply(
        &app,
        &auth,
        request,
        id,
        &body.schema,
        (body.deploy, body.dry_run),
        None,
    )
    .await
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
/// for their deployments when `deploy`; with `dry_run`, what they would
/// take only (checked as if created).
async fn apply(
    app: &AppState,
    auth: &Auth,
    request: RequestMeta,
    project_id: i64,
    output: &Value,
    (deploy, dry_run): (bool, bool),
    slug: Option<&str>,
) -> AppResult<Json<Value>> {
    let principal = auth.principal();
    // Into a project where the caller adds (and deploys) services: a role's,
    // or their role's there.
    use crate::services::access_service;
    access_service::project(app.db(), principal, project_id, "services.create").await?;
    if deploy
        && !access_service::can(
            app.db(),
            principal,
            project_id,
            permissions::SERVICES_DEPLOY,
        )
        .await?
    {
        return Err(AppError::Forbidden);
    }
    let compose_allowed = principal.can(permissions::SYSTEM_MANAGE);
    // Planned (and its passwords hashed) before the write: a project keeps
    // its name, and the write holds SQLite's whole database.
    let project = project::Entity::find_by_id(project_id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let host = (!config().service_domain.trim().is_empty()).then(|| config().service_domain.trim());
    let Plan {
        mut services,
        warnings,
        blocking,
    } = template_service::plan(output, &project.name, host).map_err(unavailable)?;
    if !dry_run {
        // Off the async workers: bcrypt takes a while per password.
        services = tokio::task::spawn_blocking(move || {
            services
                .iter_mut()
                .try_for_each(creation_service::hash_passwords)
                .map(|()| services)
        })
        .await
        .map_err(AppError::internal)??;
    }
    let txn = rustcms::db::begin_write(app.db()).await?;
    // Still there, under the name planned with (a project without
    // deployments can be renamed meanwhile): held until the commit.
    let planned_for = project.name;
    let project = project::Entity::find_by_id(project_id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if project.name != planned_for {
        return Err(AppError::Conflict {
            code: "project_renamed",
            reason: "stale",
            message: "The project was renamed meanwhile: try again",
        });
    }
    if deploy && !blocking.is_empty() {
        return Err(unavailable(format!(
            "{} Create it without deploying, and protect it before deploying.",
            blocking.join("; ")
        )));
    }
    // A wildcard domain takes every name under it: the panel's
    // administrators only.
    if services
        .iter()
        .any(|s| s.domains.iter().any(|d| d.host.starts_with("*.")))
        && !compose_allowed
    {
        return Err(AppError::Forbidden);
    }
    // A Compose file runs anything on the host, as added capabilities and
    // sysctls reach its kernel: the panel's administrators only.
    if services.iter().any(|s| {
        s.kind == "compose"
            || !s.cap_add.is_empty()
            || !s.sysctls.is_empty()
            || !s.groups.is_empty()
    }) && !compose_allowed
    {
        return Err(AppError::Forbidden);
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
        (
            services.iter().any(|s| !s.redirects.is_empty()),
            "service_redirects.create",
        ),
        (
            services.iter().any(|s| !s.basic_auth.is_empty()),
            "services.update",
        ),
    ] {
        // A role's, or the caller's role in this project.
        if wanted && !access_service::can(&txn, principal, project_id, permission).await? {
            return Err(AppError::Forbidden);
        }
    }
    if dry_run {
        drop(txn);
        let observe = access_service::can(app.db(), principal, project_id, "services.view").await?;
        let planned: Vec<_> = services
            .iter()
            .map(crate::services::capacity_service::Planned::of)
            .collect();
        let report = crate::services::capacity_service::for_new(
            app,
            project.id,
            &project.name,
            &planned,
            observe,
        )
        .await;
        return Ok(Json(json!({
            "project": project.name,
            "services": services.iter().map(|s| json!({ "name": s.name, "kind": s.kind })).collect::<Vec<_>>(),
            "warnings": warnings,
            "capacity": crate::services::capacity_service::json(&report, principal.can(permissions::SERVER_VIEW)),
        })));
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
