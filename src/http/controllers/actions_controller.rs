//! The Actions page (`/admin/api/x/apps/actions`), like EasyPanel's: what
//! the panel did lately, newest first — deployments, database backups and
//! restores, Docker actions (pulls, prunes, container actions) — each kind
//! only with its permission (Docker actions: the caller's own, every one
//! with `system.manage`).

use std::collections::HashMap;

use rustcms::prelude::*;
use rustcms::{AppResult, Json};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde_json::Value;

use crate::http::controllers::docker_controller::scope;
use crate::models::{action, backup_run, deployment, project, service};
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
    if principal.can("deployments.view_any") {
        for row in deployment::Entity::find()
            .order_by_desc(deployment::Column::Id)
            .limit(LIMIT)
            .all(db)
            .await?
        {
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
    if principal.can("database_backups.view_any") {
        for row in backup_run::Entity::find()
            .order_by_desc(backup_run::Column::Id)
            .limit(LIMIT)
            .all(db)
            .await?
        {
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
    if principal.can("docker.view") {
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
