//! The Actions page (`/admin/api/x/apps/actions`), like EasyPanel's: what
//! the panel did lately, newest first — deployments, database backups and
//! restores, Docker actions (pulls, prunes, container actions) — each kind
//! only with its permissions (deployments: `deployments.view_any` and
//! `deployments.view`, as their pages) (Docker actions: the caller's own, every one
//! with `system.manage`).

use std::collections::HashMap;

use rustcms::prelude::*;
use rustcms::{AppResult, Json};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::Value;

use crate::http::controllers::docker_controller::scope;
use crate::models::{action, backup_run, deployment, project, service};
use crate::policies::permissions;
use crate::services::docker_service::Scope;

/// How many of each kind are read.
const LIMIT: u64 = 100;

pub async fn index(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    let db = app.db();
    let principal = auth.principal();
    // Service id → (project, service), for the links.
    let services = service::Entity::find().all(db).await?;
    let projects: HashMap<i64, String> = project::Entity::find()
        .all(db)
        .await?
        .into_iter()
        .map(|p| (p.id, p.name))
        .collect();
    // Service id → its project (for what each caller may see).
    let services_of: Vec<(i64, i64)> = services.iter().map(|s| (s.id, s.project_id)).collect();
    let names: HashMap<i64, (String, String)> = services
        .into_iter()
        .map(|s| {
            let project = projects.get(&s.project_id).cloned().unwrap_or_default();
            (s.id, (project, s.name))
        })
        .collect();
    let where_ = |id: i64| {
        names
            .get(&id)
            .map(|(p, s)| json!({ "project": p, "service": s }))
            .unwrap_or(Value::Null)
    };
    let mut items: Vec<Value> = vec![];
    // The services of the projects where the caller may see `permission`
    // (`None`: every one): filtered before the limit, never after.
    use crate::services::access_service::projects_where;
    let services_where = |permission: &'static str| {
        let services = &services_of;
        async move {
            Ok::<_, rustcms::AppError>(projects_where(db, principal, permission).await?.map(
                |projects| {
                    services
                        .iter()
                        .filter(|(_, p)| projects.contains(p))
                        .map(|(s, _)| *s)
                        .collect::<Vec<i64>>()
                },
            ))
        }
    };
    let deployments_seen = services_where("deployments.view_any").await?;
    let deployments_one = services_where("deployments.view").await?;
    // Both, as their pages (each `None`: everywhere).
    let deployments_in: Option<Vec<i64>> = match (deployments_seen, deployments_one) {
        (None, None) => None,
        (Some(a), None) | (None, Some(a)) => Some(a),
        (Some(a), Some(b)) => Some(a.into_iter().filter(|s| b.contains(s)).collect()),
    };
    if deployments_in.as_ref().is_none_or(|s| !s.is_empty()) {
        let mut query = deployment::Entity::find().order_by_desc(deployment::Column::Id);
        if let Some(services) = &deployments_in {
            query = query.filter(deployment::Column::ServiceId.is_in(services.clone()));
        }
        for row in query.limit(LIMIT).all(db).await? {
            items.push(json!({
                "type": "deployment",
                "id": row.id,
                "what": row.trigger,
                "status": row.status,
                "error": row.error,
                "target": where_(row.service_id),
                "created_at": row.created_at,
                "finished_at": row.finished_at,
            }));
        }
    }
    let backups_in = services_where("database_backups.view_any").await?;
    if backups_in.as_ref().is_none_or(|s| !s.is_empty()) {
        let mut query = backup_run::Entity::find().order_by_desc(backup_run::Column::Id);
        if let Some(services) = &backups_in {
            query = query.filter(backup_run::Column::ServiceId.is_in(services.clone()));
        }
        for row in query.limit(LIMIT).all(db).await? {
            items.push(json!({
                "type": "backup",
                "id": row.id,
                "what": row.kind,
                "status": row.status,
                "error": row.error,
                "detail": row.database_name,
                "target": where_(row.service_id),
                "created_at": row.created_at,
                "finished_at": row.finished_at,
            }));
        }
    }
    if principal.can(permissions::DOCKER_VIEW) {
        let mut query = action::Entity::find().order_by_desc(action::Column::Id);
        if scope(&auth) != Scope::System {
            let own: Option<i64> = principal.id.parse().ok();
            query = query.filter(action::Column::UserId.eq(own));
        }
        for row in query.limit(LIMIT).all(db).await? {
            items.push(json!({
                "type": "docker",
                "id": row.id,
                "what": row.kind,
                "status": row.status,
                "error": row.error,
                "detail": row.target,
                "created_at": row.created_at,
                "finished_at": row.finished_at,
            }));
        }
    }
    items.sort_by(|a, b| b["created_at"].as_str().cmp(&a["created_at"].as_str()));
    items.truncate(LIMIT as usize);
    Ok(Json(json!({ "actions": items })))
}
