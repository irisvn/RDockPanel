//! Whether the server has room (`/admin/api/x/apps/capacity`): asked by
//! the panel before a service is created or deployed, started, restarted
//! or rebuilt (`capacity_service`). The server's own figures to who sees
//! the server (`server.view`); everyone else, the level and the need.

use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use sea_orm::EntityTrait;
use serde::Deserialize;
use serde_json::Value;

use crate::http::controllers::apps_controller::{read_body, service_for};
use crate::models::project;
use crate::services::capacity_service::{self as capacity, Action, Planned};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapacityBody {
    /// An existing service, and what it is asked to do…
    pub service_id: Option<i64>,
    pub action: Option<String>,
    /// …or services to create in a project.
    pub project_id: Option<i64>,
    #[serde(default)]
    pub services: Vec<PlannedBody>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedBody {
    #[serde(default)]
    pub name: String,
    pub kind: String,
    pub image: Option<String>,
    pub replicas: Option<u32>,
    pub compose: Option<String>,
}

/// `POST /capacity`: the report (`capacity_service::json`).
pub async fn check(
    State(app): State<AppState>,
    auth: Auth,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let body: CapacityBody = read_body(body)?;
    let show_host = auth.principal().can("server.view");
    let report = match (body.service_id, body.project_id) {
        (Some(id), None) => {
            let action = body
                .action
                .as_deref()
                .and_then(Action::parse)
                .ok_or_else(|| {
                    AppError::BadRequest("action: deploy, start, restart or rebuild".into())
                })?;
            let row = service_for(&app, &auth, id, "services.deploy").await?;
            let snapshot =
                crate::services::deployment_service::snapshot_without_env(app.db(), &row).await?;
            capacity::for_action(&app, &row, &snapshot, action).await
        }
        (None, Some(id)) => {
            crate::services::access_service::project(
                app.db(),
                auth.principal(),
                id,
                "services.create",
            )
            .await?;
            if body.services.is_empty() || body.services.len() > 50 {
                return Err(AppError::BadRequest("services: 1 to 50".into()));
            }
            let project = project::Entity::find_by_id(id)
                .one(app.db())
                .await?
                .ok_or(AppError::NotFound)?;
            let planned: Vec<Planned> = body
                .services
                .into_iter()
                .map(|s| Planned {
                    name: s.name,
                    kind: s.kind,
                    image: s.image,
                    replicas: s.replicas.unwrap_or(1).clamp(1, 50),
                    compose: s.compose,
                    ..Planned::default()
                })
                .collect();
            let observe = crate::services::access_service::can(
                app.db(),
                auth.principal(),
                project.id,
                "services.view",
            )
            .await?;
            capacity::for_new(&app, project.id, &project.name, &planned, observe).await
        }
        _ => {
            return Err(AppError::BadRequest(
                "a service_id and action, or a project_id and services".into(),
            ));
        }
    };
    Ok(Json(capacity::json(&report, show_host)))
}
