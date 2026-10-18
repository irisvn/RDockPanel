//! Database backups in the apps interface (`/admin/api/x/apps`): a database
//! service's backup configurations and runs, a run by hand, a restore, a
//! provider's files (to pick one to restore) and its "Test" button. The
//! configurations and providers themselves are written through their
//! resources (`database_backups`, `storage_providers`).

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::Deserialize;
use serde_json::Value;

use crate::models::{backup_run, database_backup, service, storage_provider};
use crate::services::backup_service::{self, supported, valid_database};
use crate::services::database_service::Engine;
use crate::services::storage_service::{Store, valid_key};

/// Running a backup by hand.
pub const RUN_PERMISSION: &str = "backups.run";
/// Restoring one (it replaces the database's data).
pub const RESTORE_PERMISSION: &str = "backups.restore";

fn audit_context(auth: &Auth, request: RequestMeta) -> AuditContext {
    AuditContext::new(AuditActor::user(auth.principal()), request)
}

fn user_id(auth: &Auth) -> Option<i64> {
    auth.principal().id.parse().ok()
}

/// A run without its log (lists).
fn summary(run: &backup_run::Model) -> Value {
    json!({
        "id": run.id,
        "backup_id": run.backup_id,
        "kind": run.kind,
        "trigger": run.trigger,
        "status": run.status,
        "database_name": run.database_name,
        "volume": run.volume,
        "file": run.file,
        "size": run.size,
        "error": run.error,
        "started_at": run.started_at,
        "finished_at": run.finished_at,
        "created_at": run.created_at,
    })
}

/// The paths of a service's named volumes in its container.
async fn volume_paths(app: &AppState, id: i64) -> AppResult<Vec<String>> {
    use crate::models::service_mount;
    Ok(service_mount::Entity::find()
        .filter(service_mount::Column::ServiceId.eq(id))
        .filter(service_mount::Column::Kind.eq("volume"))
        .all(app.db())
        .await?
        .into_iter()
        .map(|m| m.target)
        .collect())
}

/// A database service's Backups tab: its configurations, its last runs, the
/// providers to choose from.
pub async fn index(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = service::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let volumes = volume_paths(&app, row.id).await?;
    let db = app.db();
    let configs = database_backup::Entity::find()
        .filter(database_backup::Column::ServiceId.eq(row.id))
        .order_by_asc(database_backup::Column::Id)
        .all(db)
        .await?;
    let runs = backup_run::Entity::find()
        .filter(backup_run::Column::ServiceId.eq(row.id))
        .order_by_desc(backup_run::Column::Id)
        .limit(30)
        .all(db)
        .await?;
    let providers = if auth.principal().can("storage_providers.view_any") {
        storage_provider::Entity::find()
            .order_by_asc(storage_provider::Column::Name)
            .all(db)
            .await?
            .into_iter()
            .map(|p| json!({ "id": p.id, "name": p.name, "kind": p.kind }))
            .collect()
    } else {
        vec![]
    };
    Ok(Json(json!({
        "supported": Engine::parse(&row.kind).is_some_and(supported),
        "volumes": volumes,
        "configs": configs.iter().map(|c| json!({
            "id": c.id,
            "storage_provider_id": c.storage_provider_id,
            "database_name": c.database_name,
            "volume": c.volume,
            "enabled": c.enabled,
            "schedule": c.schedule,
            "retention": c.retention,
            "path": c.path,
            "last_slot_at": c.last_slot_at,
        })).collect::<Vec<_>>(),
        "runs": runs.iter().map(summary).collect::<Vec<_>>(),
        "providers": providers,
    })))
}

/// One run with its log.
pub async fn show_run(State(app): State<AppState>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let run = backup_run::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let mut body = summary(&run);
    body["log"] = json!(run.log.unwrap_or_default());
    body["service_id"] = json!(run.service_id);
    Ok(Json(body))
}

/// Runs a configuration now ("Manual Run"): 202 with the run.
pub async fn run(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let config = database_backup::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    let run = backup_service::request(&txn, &config, "manual", user_id(&auth)).await?;
    let entry = AuditEntry::new("backups.run", "database_backups")
        .key(id.to_string())
        .meta(json!({ "run": run.id }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(summary(&run))))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreBody {
    pub storage_provider_id: i64,
    /// The file in the provider, e.g. `production/shop/shop-20261019T020000Z.sql.gz`.
    pub file: String,
    /// The database restored into (for a volume: its name, a label).
    pub database: String,
    /// A volume's path in the container: a volume restore.
    pub volume: Option<String>,
}

/// Restores a file into the service's database (in place): 202 with the run.
pub async fn restore(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<RestoreBody>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let row = service::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let mut errors = ValidationErrors::new();
    let volume = body
        .volume
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let file = body.file.trim().trim_start_matches('/');
    match volume {
        Some(path) => {
            if !volume_paths(&app, row.id).await?.iter().any(|v| v == path) {
                errors.add("volume", "is not a volume of this service");
            }
            if !valid_key(file) || !file.ends_with(".tar.gz") {
                errors.add("file", "must be a volume backup of the provider (.tar.gz)");
            }
        }
        None => {
            if !Engine::parse(&row.kind).is_some_and(supported) {
                errors.add(
                    "database",
                    "this service has no database restored by the panel",
                );
            }
            if !valid_key(file) || !(file.ends_with(".sql.gz") || file.ends_with(".archive.gz")) {
                errors.add(
                    "file",
                    "must be a backup file of the provider (.sql.gz or .archive.gz)",
                );
            }
        }
    }
    if !valid_database(body.database.trim()) {
        errors.add("database", "is not a database name");
    }
    let provider = storage_provider::Entity::find_by_id(body.storage_provider_id)
        .one(app.db())
        .await?;
    if provider.is_none() {
        errors.add("storage_provider_id", "is not a storage provider");
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }
    let provider = provider.ok_or(AppError::NotFound)?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    let run = backup_service::request_restore(
        &txn,
        &row,
        &provider,
        file,
        body.database.trim(),
        volume,
        user_id(&auth),
    )
    .await?;
    let entry = AuditEntry::new("backups.restore", "services")
        .key(id.to_string())
        .meta(json!({ "run": run.id, "file": file, "database": body.database.trim() }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(summary(&run))))
}

#[derive(Debug, Deserialize)]
pub struct FilesQuery {
    pub path: String,
}

/// The files in a folder of a provider (newest first): what can be restored.
pub async fn files(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<FilesQuery>,
) -> AppResult<Json<Value>> {
    let provider = storage_provider::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let folder = query.path.trim().trim_matches('/');
    if !valid_key(folder) {
        return Err(ValidationErrors::field("path", "is not a folder of the provider").into());
    }
    let store = Store::open(&provider, app.keyring()).map_err(AppError::BadRequest)?;
    let mut files = store.list(folder).await.map_err(AppError::BadRequest)?;
    files.reverse();
    Ok(Json(json!({
        "files": files.iter().map(|(key, size)| json!({ "file": key, "size": size })).collect::<Vec<_>>(),
    })))
}

/// The "Test" button: writes, lists and deletes a small file.
pub async fn check(State(app): State<AppState>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let provider = storage_provider::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let result = match Store::open(&provider, app.keyring()) {
        Ok(store) => store.check().await,
        Err(e) => Err(e),
    };
    Ok(Json(match result {
        Ok(()) => json!({ "ok": true }),
        Err(error) => json!({ "ok": false, "error": error }),
    }))
}

/// The storage providers (Settings): never their secret key, only whether
/// one is set.
pub async fn providers(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let rows = storage_provider::Entity::find()
        .order_by_asc(storage_provider::Column::Name)
        .all(app.db())
        .await?;
    Ok(Json(json!({
        "providers": rows.iter().map(|p| json!({
            "id": p.id,
            "name": p.name,
            "kind": p.kind,
            "local_path": p.local_path,
            "endpoint": p.endpoint,
            "region": p.region,
            "bucket": p.bucket,
            "access_key": p.access_key,
            "path_style": p.path_style,
            "secret_set": p.secret_key.is_some(),
        })).collect::<Vec<_>>(),
    })))
}
