//! The public API (`/api/v1`, `Authorization: Bearer rcms_…`; docs/api.md):
//! projects and services, deployments (deploy, restart, their status and
//! log), environments and domains — as the token's user may, within its
//! scopes (`api_token_service`). Errors are RustCMS's JSON (`code`,
//! `message`).

use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, TransactionTrait};
use serde_json::Value;

use crate::models::{deployment, project, service, service_domain};
use crate::policies::permissions;
use crate::services::api_token_service::{ApiCaller, caller};
use crate::services::deployment_service::{self, Trigger, summary};

async fn service_json(
    db: &sea_orm::DatabaseConnection,
    who: &ApiCaller,
    row: &service::Model,
    project: &str,
) -> AppResult<Value> {
    // Each related record with its own permission (and the token's scope).
    let domains: Vec<Value> = if who
        .can(db, row.project_id, "service_domains.view_any")
        .await?
    {
        service_domain::Entity::find()
            .filter(service_domain::Column::ServiceId.eq(row.id))
            .order_by_asc(service_domain::Column::Id)
            .all(db)
            .await?
            .into_iter()
            .map(|d| json!({ "host": d.host, "path": d.path, "port": d.port, "https": d.https }))
            .collect()
    } else {
        vec![]
    };
    let latest = if who.can(db, row.project_id, "deployments.view").await? {
        deployment_service::latest(db, row.id).await?.map(|d| {
            let mut s = summary(&d);
            s["log"] = Value::Null;
            s
        })
    } else {
        None
    };
    Ok(json!({
        "id": row.id,
        "project": project,
        "name": row.name,
        "kind": row.kind,
        "image": row.image,
        "status": row.status,
        "replicas": row.replicas,
        "busy": row.current_deployment_id.is_some(),
        "domains": domains,
        "latest_deployment": latest,
    }))
}

/// `GET /me`: the token's user and scopes (any API token: `rdp login`
/// checks a token with it).
pub async fn me(
    State(app): State<AppState>,
    auth: Auth,
    _limit: Throttle<120>,
) -> AppResult<Json<Value>> {
    let who = caller(&app, &auth).await?;
    Ok(Json(json!({
        "user": who.principal.id,
        "scopes": crate::services::api_token_service::scopes_of(&who.abilities),
    })))
}

/// `GET /projects`: the projects the caller sees, with their services.
pub async fn projects(
    State(app): State<AppState>,
    auth: Auth,
    _limit: Throttle<120>,
) -> AppResult<Json<Value>> {
    let who = caller(&app, &auth).await?;
    let db = app.db();
    if !who.scoped("projects.view_any") {
        return Err(AppError::Forbidden);
    }
    let mut out = vec![];
    for p in project::Entity::find()
        .order_by_asc(project::Column::Name)
        .all(db)
        .await?
    {
        if !who.can(db, p.id, "projects.view_any").await? {
            continue;
        }
        let mut services = vec![];
        if who.can(db, p.id, "services.view_any").await? {
            for s in service::Entity::find()
                .filter(service::Column::ProjectId.eq(p.id))
                .order_by_asc(service::Column::Name)
                .all(db)
                .await?
            {
                services.push(json!({
                    "id": s.id,
                    "name": s.name,
                    "kind": s.kind,
                    "status": s.status,
                }));
            }
        }
        out.push(json!({ "id": p.id, "name": p.name, "services": services }));
    }
    Ok(Json(json!({ "projects": out })))
}

/// A record the caller may not see is answered as missing.
fn hidden(err: AppError) -> AppError {
    match err {
        AppError::Forbidden => AppError::NotFound,
        other => other,
    }
}

/// The service `project/service` or a numeric id, for `permission`.
async fn find(
    app: &AppState,
    who: &ApiCaller,
    id: &str,
    permission: &str,
) -> AppResult<(service::Model, String)> {
    let db = app.db();
    if !who.scoped(permission) {
        return Err(AppError::Forbidden);
    }
    let service_id = match id.split_once('/') {
        Some((project, name)) => {
            let project = project::Entity::find()
                .filter(project::Column::Name.eq(project))
                .one(db)
                .await?;
            let found = match project {
                Some(p) => {
                    service::Entity::find()
                        .filter(service::Column::ProjectId.eq(p.id))
                        .filter(service::Column::Name.eq(name))
                        .one(db)
                        .await?
                }
                None => None,
            };
            // Unseen or missing: the same answer.
            match found {
                Some(s) => s.id,
                None => return Err(AppError::NotFound),
            }
        }
        None => id.parse::<i64>().map_err(|_| AppError::NotFound)?,
    };
    // Another project's, or none: the same answer (no existence oracle).
    let row = who
        .service(db, service_id, permission)
        .await
        .map_err(hidden)?;
    let project = project::Entity::find_by_id(row.project_id)
        .one(db)
        .await?
        .map(|p| p.name)
        .unwrap_or_default();
    Ok((row, project))
}

/// `GET /services/{id}` (`id`: a number, or `project/service` URL-encoded).
pub async fn show(
    State(app): State<AppState>,
    auth: Auth,
    _limit: Throttle<120>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let who = caller(&app, &auth).await?;
    let (row, project) = find(&app, &who, &id, "services.view").await?;
    Ok(Json(
        json!({ "service": service_json(app.db(), &who, &row, &project).await? }),
    ))
}

/// Queues `trigger` for the service: the deployment (202).
async fn queue(
    app: &AppState,
    who: &ApiCaller,
    row: &service::Model,
    trigger: Trigger,
) -> AppResult<Value> {
    use sea_orm::QuerySelect;
    let txn = app.db().begin().await?;
    // Read again, locked: what is deployed is what is authorized (a move to
    // another project or new privileges meanwhile waits, or went first).
    let row = &service::Entity::find_by_id(row.id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if !who
        .can(&txn, row.project_id, permissions::SERVICES_DEPLOY)
        .await?
    {
        return Err(AppError::NotFound);
    }
    if row.kind == "compose" || crate::models::service::privileged(row) {
        crate::http::controllers::apps_controller::forbid_privileged_of(&who.principal, row)?;
    }
    let user = who.principal.id.parse().ok();
    let queued = deployment_service::request(&txn, row, trigger, None, user, app.keyring()).await?;
    rustcms::queue::dispatch(
        &txn,
        &crate::jobs::run_deployment::RunDeployment {
            deployment_id: queued.id,
        },
    )
    .await?;
    txn.commit().await?;
    Ok(summary(&queued))
}

/// `POST /services/{id}/deploy`.
pub async fn deploy(
    State(app): State<AppState>,
    auth: Auth,
    _limit: Throttle<10>,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let who = caller(&app, &auth).await?;
    let (row, _) = find(&app, &who, &id, permissions::SERVICES_DEPLOY).await?;
    let queued = queue(&app, &who, &row, Trigger::Deploy).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "deployment": queued }))))
}

/// `POST /services/{id}/restart`.
pub async fn restart(
    State(app): State<AppState>,
    auth: Auth,
    _limit: Throttle<10>,
    Path(id): Path<String>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let who = caller(&app, &auth).await?;
    let (row, _) = find(&app, &who, &id, permissions::SERVICES_DEPLOY).await?;
    let queued = queue(&app, &who, &row, Trigger::Restart).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "deployment": queued }))))
}

/// `GET /deployments/{id}`: its status, and its log (`deployments.view`).
pub async fn deployment(
    State(app): State<AppState>,
    auth: Auth,
    _limit: Throttle<120>,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let who = caller(&app, &auth).await?;
    if !who.scoped("deployments.view") {
        return Err(AppError::Forbidden);
    }
    let row = deployment::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    who.service(app.db(), row.service_id, "deployments.view")
        .await
        .map_err(hidden)?;
    Ok(Json(json!({ "deployment": summary(&row) })))
}

/// `GET /services/{id}/env`: its variables in clear (`secrets` scope;
/// audited as the panel's own read).
pub async fn env(
    State(app): State<AppState>,
    auth: Auth,
    _limit: Throttle<120>,
    request: rustcms::audit::RequestMeta,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let who = caller(&app, &auth).await?;
    let (row, _) = find(&app, &who, &id, permissions::SERVICES_SECRETS).await?;
    let env =
        crate::services::deployment_service::service_env(&row, app.keyring()).map_err(|_| {
            AppError::BadRequest(
                "the environment cannot be read (sealed with another APP_KEY)".into(),
            )
        })?;
    let entry = rustcms::audit::AuditEntry::new("services.env_read", "services")
        .key(row.id.to_string())
        .meta(json!({ "names": env.keys().collect::<Vec<_>>(), "via": "api" }));
    let context = rustcms::audit::AuditContext::new(
        rustcms::audit::AuditActor::user(&who.principal),
        request,
    );
    // No audit, no values (as the panel's own read).
    let txn = rustcms::db::begin_write(app.db()).await?;
    rustcms::audit::record(&txn, &context, entry).await?;
    txn.commit().await?;
    Ok(Json(json!({ "env": env })))
}

/// `GET /services/{id}/domains`.
pub async fn domains(
    State(app): State<AppState>,
    auth: Auth,
    _limit: Throttle<120>,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let who = caller(&app, &auth).await?;
    let (row, project) = find(&app, &who, &id, "service_domains.view_any").await?;
    Ok(Json(
        json!({ "domains": service_json(app.db(), &who, &row, &project).await?["domains"] }),
    ))
}
