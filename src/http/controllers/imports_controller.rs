//! Importing existing containers (`routes/admin.rs`, under
//! `/admin/api/x/imports`): the sources Docker lists, an import's plan, and
//! what the admin chooses in it. The panel's administrators only
//! (`system.manage`). Nothing here carries a secret: the plan has names,
//! ports and findings, and raw inspects stay in the job that reads them.

use std::collections::BTreeMap;

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::queue;

use rustcms::{AppError, AppResult, Json, ValidationErrors};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel, QuerySelect, Set};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::jobs::import_discover::ImportDiscover;
use crate::models::container_import;
use crate::services::deploy_rules_service::{reserved_name, valid_name};
use crate::services::docker_service::Docker;

const PAGE: usize = 25;

fn docker(app: &AppState) -> AppResult<std::sync::Arc<Docker>> {
    app.service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not configured"))
}

fn invalid(field: &str, message: &str) -> AppError {
    AppError::Validation(ValidationErrors::field(field, message))
}

#[derive(Deserialize)]
pub struct Paging {
    page: Option<usize>,
}

/// The containers Docker lists, grouped by Compose project (25 groups a
/// page). Only names, states and the Compose labels: a list's other labels
/// can hold secrets.
pub async fn sources(
    State(app): State<AppState>,
    Query(paging): Query<Paging>,
) -> AppResult<Json<Value>> {
    let listed = docker(&app)?
        .engine()
        .containers_detailed()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for item in &listed {
        let label = |name: &str| item["Labels"][name].as_str().map(str::to_owned);
        let system = item["Labels"]["rdockpanel.system"] == "true"
            || item["Labels"]["rdockpanel.managed"] == "true"
            || item["Labels"]["com.docker.swarm.service.id"].is_string();
        let name = item["Names"][0]
            .as_str()
            .unwrap_or_default()
            .trim_start_matches('/')
            .to_owned();
        // A container of no Compose project is a group of its own.
        let project = label("com.docker.compose.project").unwrap_or_else(|| name.clone());
        groups.entry(project).or_default().push(json!({
            "id": item["Id"],
            "name": name,
            "image": item["Image"],
            "state": item["State"],
            "compose_service": label("com.docker.compose.service"),
            "excluded": system,
        }));
    }
    let total = groups.len();
    let page = paging.page.unwrap_or(1).max(1);
    let shown: Vec<Value> = groups
        .into_iter()
        .skip((page - 1) * PAGE)
        .take(PAGE)
        .map(|(project, containers)| {
            let compose = containers.iter().any(|c| c["compose_service"].is_string());
            json!({ "project": project, "compose": compose, "containers": containers })
        })
        .collect();
    Ok(Json(
        json!({ "groups": shown, "total": total, "page": page, "per_page": PAGE }),
    ))
}

#[derive(Deserialize)]
pub struct NewImport {
    /// `container` or `project`.
    source_kind: String,
    source_ref: String,
    project_name: Option<String>,
}

/// Starts reading a container, or a Compose project: 202, the plan comes
/// from the `import.discover` job (the audit entry and the job are one write).
pub async fn create(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(new): Json<NewImport>,
) -> AppResult<(StatusCode, Json<Value>)> {
    if !matches!(new.source_kind.as_str(), "container" | "project") {
        return Err(invalid("source_kind", "must be `container` or `project`"));
    }
    if new.source_ref.is_empty() || new.source_ref.len() > 255 {
        return Err(invalid("source_ref", "is required"));
    }
    let project_name = new.project_name.unwrap_or_else(|| {
        // The Compose project's own name, made one the panel takes.
        new.source_ref
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .chars()
            .take(40)
            .collect()
    });
    check_project_name(&project_name)?;
    let principal = auth.principal();
    let txn = rustcms::db::begin_write(app.db()).await?;
    let now = chrono::Utc::now();
    let row = container_import::ActiveModel {
        project_name: Set(project_name.clone()),
        source_kind: Set(new.source_kind.clone()),
        source_ref: Set(new.source_ref.clone()),
        state: Set("discovering".to_owned()),
        auto_undo: Set(true),
        undo_requested: Set(false),
        created_by: Set(crate::services::access_service::user_of(principal)),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    let entry = AuditEntry::new("imports.discover", "container_imports")
        .key(row.id.to_string())
        .meta(json!({ "source_kind": new.source_kind, "source": new.source_ref }));
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    queue::dispatch(&txn, &ImportDiscover { import_id: row.id }).await?;
    txn.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": row.id }))))
}

fn check_project_name(name: &str) -> AppResult<()> {
    if !valid_name(name) {
        return Err(invalid(
            "project_name",
            "must be 1-40 lowercase letters, digits and dashes, starting with a letter",
        ));
    }
    if reserved_name(name) {
        return Err(invalid("project_name", "is reserved for the panel"));
    }
    Ok(())
}

fn view(row: &container_import::Model) -> Value {
    let plan: Value = row
        .plan
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
        .unwrap_or(Value::Null);
    json!({
        "id": row.id,
        "state": row.state,
        "source_kind": row.source_kind,
        "source_ref": row.source_ref,
        "project_name": row.project_name,
        "error": row.error,
        "plan": plan,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    })
}

async fn find(app: &AppState, id: i64) -> AppResult<container_import::Model> {
    container_import::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)
}

pub async fn show(State(app): State<AppState>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    use crate::models::container_import_volume;
    use sea_orm::{ColumnTrait, QueryFilter, QueryOrder};
    let row = find(&app, id).await?;
    let volumes = container_import_volume::Entity::find()
        .filter(container_import_volume::Column::ImportId.eq(id))
        .order_by_asc(container_import_volume::Column::Id)
        .all(app.db())
        .await?;
    let mut shown = view(&row);
    shown["volumes"] = json!(
        volumes
            .iter()
            .map(|v| json!({
                "id": v.id,
                "original_name": v.original_name,
                "mode": v.mode,
                "panel_volume": v.panel_volume,
                "copied_at": v.copied_at,
                "released_at": v.released_at,
            }))
            .collect::<Vec<_>>()
    );
    Ok(Json(shown))
}

#[derive(Deserialize)]
pub struct PlanChange {
    /// Container id → `bring` or `leave`.
    choices: Option<BTreeMap<String, String>>,
    project_name: Option<String>,
    /// Leaving a container behind breaks calls to it: said, and confirmed.
    confirm: Option<bool>,
}

/// The admin's choices in the plan: which containers to bring, the
/// project's name. A hard edge cannot be cut; a soft one only with `confirm`.
pub async fn change_plan(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(change): Json<PlanChange>,
) -> AppResult<Json<Value>> {
    if let Some(name) = &change.project_name {
        check_project_name(name)?;
    }
    let principal = auth.principal();
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = container_import::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if row.state != "planned" {
        return Err(AppError::Conflict {
            code: "import_not_planned",
            reason: "state",
            message: "Only a plan that is ready can be changed",
        });
    }
    let mut plan: Value = row
        .plan
        .as_deref()
        .and_then(|p| serde_json::from_str(p).ok())
        .ok_or(AppError::NotFound)?;
    let mut cut_soft = false;
    if let Some(choices) = &change.choices {
        let items = plan["items"].as_array_mut().ok_or(AppError::NotFound)?;
        for (container, choice) in choices {
            if !matches!(choice.as_str(), "bring" | "leave") {
                return Err(invalid("choices", "must be `bring` or `leave`"));
            }
            let item = items
                .iter_mut()
                .find(|i| i["container_id"] == *container)
                .ok_or_else(|| invalid("choices", "names a container that is not in the plan"))?;
            if item["choice"] == "blocked" {
                return Err(invalid("choices", "a blocked container cannot be chosen"));
            }
            item["choice"] = json!(choice);
        }
        let chosen = |plan: &Value, id: &Value| -> bool {
            plan["items"]
                .as_array()
                .and_then(|items| items.iter().find(|i| i["container_id"] == *id))
                .is_some_and(|i| i["choice"] == "bring")
        };
        for edge in plan["edges"].as_array().into_iter().flatten() {
            if chosen(&plan, &edge["from"]) == chosen(&plan, &edge["to"]) {
                continue;
            }
            match edge["hardness"].as_str() {
                Some("hard" | "block") => {
                    return Err(invalid(
                        "choices",
                        "a container that shares data or a network with another goes with it",
                    ));
                }
                Some("soft") => cut_soft = true,
                _ => {}
            }
        }
    }
    if cut_soft && change.confirm != Some(true) {
        return Err(invalid(
            "confirm",
            "leaving this container behind breaks calls to it: confirm to go on",
        ));
    }
    let mut changed = row.clone().into_active_model();
    if let Some(name) = &change.project_name {
        changed.project_name = Set(name.clone());
        plan["project_name"] = json!(name);
    }
    changed.plan = Set(Some(plan.to_string()));
    changed.updated_at = Set(chrono::Utc::now());
    let saved = changed.update(&txn).await?;
    let entry = AuditEntry::new("imports.plan", "container_imports")
        .key(id.to_string())
        .meta(json!({ "choices": change.choices, "project_name": change.project_name }));
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok(Json(view(&saved)))
}

/// Reads again: the plan is made anew from what Docker says now.
pub async fn rescan(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let principal = auth.principal();
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = container_import::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if !container_import::EDITABLE.contains(&row.state.as_str()) {
        return Err(AppError::Conflict {
            code: "import_not_editable",
            reason: "state",
            message: "This import is under way: it cannot be read again",
        });
    }
    let mut again = row.into_active_model();
    again.state = Set("discovering".to_owned());
    again.error = Set(None);
    again.updated_at = Set(chrono::Utc::now());
    again.update(&txn).await?;
    let entry = AuditEntry::new("imports.discover", "container_imports").key(id.to_string());
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    queue::dispatch(&txn, &ImportDiscover { import_id: id }).await?;
    txn.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
}

/// Deletes an import that did not start (its plan, nothing else).
pub async fn destroy(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let principal = auth.principal();
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = container_import::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if !container_import::EDITABLE.contains(&row.state.as_str()) {
        return Err(AppError::Conflict {
            code: "import_not_editable",
            reason: "state",
            message: "This import is under way: it cannot be deleted",
        });
    }
    container_import::Entity::delete_by_id(id)
        .exec(&txn)
        .await?;
    let entry = AuditEntry::new("imports.delete", "container_imports").key(id.to_string());
    audit::record(
        &txn,
        &AuditContext::new(AuditActor::user(principal), request),
        entry,
    )
    .await?;
    txn.commit().await?;
    Ok(Json(json!({ "deleted": id })))
}

#[derive(Deserialize)]
pub struct Start {
    /// The project's name, typed to confirm.
    confirm: String,
}

/// Starts the import: 202, the steps run in the background.
pub async fn start(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<Start>,
) -> AppResult<(StatusCode, Json<Value>)> {
    crate::services::import_service::start(&app, auth.principal(), request, id, &body.confirm)
        .await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
}

/// Asks to undo: the step that holds the import sees it and undoes at its
/// next look; an import that waits is undone at once.
pub async fn undo(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<(StatusCode, Json<Value>)> {
    crate::services::import_run_service::ask_undo(&app, auth.principal(), request, id).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
}

macro_rules! action {
    ($name:ident, $call:path, $what:expr) => {
        pub async fn $name(
            State(app): State<AppState>,
            auth: Auth,
            request: RequestMeta,
            Path(id): Path<i64>,
        ) -> AppResult<(StatusCode, Json<Value>)> {
            $call(&app, auth.principal(), request, id, $what).await?;
            Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
        }
    };
}

action!(retry, crate::services::import_run_service::resume, "retry");
action!(
    continue_on,
    crate::services::import_run_service::resume,
    "continue"
);

/// Takes the services as they are, when every one runs.
pub async fn accept(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    crate::services::import_run_service::accept(&app, auth.principal(), request, id).await?;
    Ok(Json(json!({ "id": id })))
}

/// Ends the import: the originals are removed.
pub async fn finish(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    crate::services::import_run_service::finish(&app, auth.principal(), request, id).await?;
    Ok(Json(json!({ "id": id })))
}

/// Stops the originals that run again.
pub async fn stop_originals(
    State(app): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let stopped = crate::services::import_run_service::stop_originals(&app, id).await?;
    Ok(Json(json!({ "stopped": stopped })))
}

pub async fn stop_watching(
    State(app): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    crate::services::import_run_service::stop_watching(&app, id).await?;
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize, Default)]
pub struct Release {
    /// Remove the original volume too.
    #[serde(default)]
    remove: bool,
}

pub async fn release_volume(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path((id, volume)): Path<(i64, i64)>,
    Json(body): Json<Release>,
) -> AppResult<Json<Value>> {
    crate::services::import_run_service::release_volume(
        &app,
        auth.principal(),
        request,
        id,
        volume,
        body.remove,
    )
    .await?;
    Ok(Json(json!({ "released": volume })))
}

#[derive(Deserialize)]
pub struct MoveData {
    /// The service's name, typed to confirm.
    confirm: String,
}

/// Moves the folders a service uses in place into volumes of the panel's: 202.
pub async fn move_data(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<MoveData>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let import = crate::services::import_move_service::start(
        &app,
        auth.principal(),
        request,
        id,
        &body.confirm,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": import }))))
}

/// Puts the folders back as the mounts (what was written to the volumes stays).
pub async fn undo_move(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let import =
        crate::services::import_move_service::undo(&app, auth.principal(), request, id).await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": import }))))
}

#[derive(Deserialize)]
pub struct CleanFolder {
    folder: String,
    /// The folder's name, typed to confirm.
    confirm: String,
}

/// Deletes the old folder of a service whose data moved to a volume.
pub async fn clean_old_folder(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<CleanFolder>,
) -> AppResult<Json<Value>> {
    crate::services::import_move_service::clean_old_folder(
        &app,
        auth.principal(),
        request,
        id,
        &body.folder,
        &body.confirm,
    )
    .await?;
    Ok(Json(json!({ "cleaned": body.folder })))
}
