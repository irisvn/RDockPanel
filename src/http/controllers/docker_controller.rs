//! The Docker pages' endpoints (`routes/admin.rs`, under
//! `/admin/api/x/docker`). Each route's permission is checked before these
//! run; here, the panel's own objects need `system.manage` on top
//! ([`scope`]), and every change is written to the actions log.

use futures_util::StreamExt;
use rustcms::audit::{self, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::queue;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use serde::Deserialize;
use serde_json::Value;

use crate::jobs::pull_image::PullImage;
use crate::models::action;
use crate::services::action_service::{self, Initiator};
use crate::services::docker_service::{ContainerAction, Docker, LogQuery, Scope};

/// Sees and touches the panel's own containers, volumes and networks.
pub const SYSTEM_PERMISSION: &str = "system.manage";

/// Lines a log request returns at most, and its bytes (one line is cut to
/// `MAX_LOG_LINE_BYTES`).
const MAX_LOG_LINES: u32 = 5_000;
const MAX_LOG_BYTES: usize = 2 * 1024 * 1024;
const MAX_LOG_LINE_BYTES: usize = 16 * 1024;

/// The first `max` bytes of `text`, on a character boundary.
fn cut(text: &str, max: usize) -> &str {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

pub fn scope(auth: &Auth) -> Scope {
    if auth.principal().can(SYSTEM_PERMISSION) {
        Scope::System
    } else {
        Scope::User
    }
}

fn docker(app: &AppState) -> AppResult<std::sync::Arc<Docker>> {
    app.service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not bound"))
}

fn initiator(auth: &Auth, request: RequestMeta) -> Initiator {
    Initiator::user(auth.principal(), request)
}

pub async fn info(State(app): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!(docker(&app)?.info().await?)))
}

pub async fn containers(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    Ok(Json(
        json!({ "containers": docker(&app)?.containers(scope(&auth)).await? }),
    ))
}

pub async fn container(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    Ok(Json(
        docker(&app)?.inspect_container(&id, scope(&auth)).await?,
    ))
}

pub async fn container_action(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path((id, action)): Path<(String, ContainerAction)>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let kind = format!("container.{}", json!(action).as_str().unwrap_or_default());
    let by = initiator(&auth, request);
    action_service::run(
        app.db(),
        &by,
        &kind,
        &id,
        docker.container_action(&id, action, scope(&auth)),
    )
    .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Force {
    #[serde(default)]
    pub force: bool,
}

pub async fn remove_container(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<String>,
    Query(Force { force }): Query<Force>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = initiator(&auth, request);
    action_service::run(
        app.db(),
        &by,
        "container.remove",
        &id,
        docker.remove_container(&id, force, scope(&auth)),
    )
    .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogParams {
    pub tail: Option<u32>,
    /// Unix seconds.
    pub since: Option<i64>,
    #[serde(default)]
    pub timestamps: bool,
}

/// The last lines of a container's output (live streaming comes with the
/// framework's server-sent events).
pub async fn logs(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(params): Query<LogParams>,
) -> AppResult<Json<Value>> {
    let tail = params.tail.unwrap_or(500).min(MAX_LOG_LINES);
    let query = LogQuery {
        follow: false,
        tail: Some(tail),
        since: params.since,
        timestamps: params.timestamps,
    };
    let mut stream = docker(&app)?.logs(&id, query, scope(&auth)).await?;
    let (mut lines, mut bytes, mut truncated) = (Vec::new(), 0, false);
    while let Some(line) = stream.next().await {
        let mut line = line?;
        if line.text.len() > MAX_LOG_LINE_BYTES {
            line.text = cut(&line.text, MAX_LOG_LINE_BYTES).to_owned();
            truncated = true;
        }
        bytes += line.text.len();
        lines.push(line);
        if bytes > MAX_LOG_BYTES || lines.len() >= tail as usize * 2 {
            truncated = true;
            break;
        }
    }
    Ok(Json(json!({ "lines": lines, "truncated": truncated })))
}

pub async fn images(State(app): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!({ "images": docker(&app)?.images().await? })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pull {
    pub image: String,
}

/// Queues the pull; its progress is the log of the returned action.
pub async fn pull_image(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(Pull { image }): Json<Pull>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let image = image.trim().to_owned();
    if !valid_image_reference(&image) {
        return Err(AppError::Validation(ValidationErrors::field(
            "image",
            "is not an image reference",
        )));
    }
    let by = initiator(&auth, request);
    let txn = rustcms::db::begin_write(app.db()).await?;
    let row = action_service::start(&txn, &by, "image.pull", &image, action::QUEUED).await?;
    let entry = AuditEntry::new("docker.image.pull", "docker")
        .key(&image)
        .meta(json!({ "action": row.id }));
    audit::record(&txn, &by.audit, entry).await?;
    queue::dispatch_with(
        &txn,
        &PullImage {
            action_id: row.id,
            image,
        },
        None,
        by.user_id,
    )
    .await?;
    txn.commit().await?;
    Ok((StatusCode::ACCEPTED, Json(json!({ "action": row }))))
}

pub async fn remove_image(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<String>,
    Query(Force { force }): Query<Force>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = initiator(&auth, request);
    action_service::run(
        app.db(),
        &by,
        "image.remove",
        &id,
        docker.remove_image(&id, force, scope(&auth)),
    )
    .await?;
    Ok(Json(json!({ "ok": true })))
}

/// One entry of the actions log (a pull's progress and outcome).
/// A user reads their own actions; all of them need `system.manage` (they
/// may concern the panel's own objects and carry Docker's raw errors).
pub async fn action(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    use sea_orm::EntityTrait;
    let row = action::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let own = row.user_id.is_some() && row.user_id == auth.principal().id.parse().ok();
    if !own && scope(&auth) != Scope::System {
        return Err(AppError::NotFound);
    }
    Ok(Json(json!({ "action": row })))
}

/// Stopped containers and dangling images, never the panel's own.
pub async fn prune(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = initiator(&auth, request);
    let report =
        action_service::run(app.db(), &by, "system.prune", "docker", docker.prune()).await?;
    Ok(Json(json!({ "report": report })))
}

pub async fn volumes(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    Ok(Json(
        json!({ "volumes": docker(&app)?.volumes(scope(&auth)).await? }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewVolume {
    pub name: String,
}

pub async fn create_volume(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(NewVolume { name }): Json<NewVolume>,
) -> AppResult<(StatusCode, Json<Value>)> {
    if !valid_object_name(&name) {
        return Err(AppError::Validation(ValidationErrors::field(
            "name",
            "must be 1-128 letters, digits, `_`, `.`, `-`",
        )));
    }
    let docker = docker(&app)?;
    let by = initiator(&auth, request);
    let volume = action_service::run(
        app.db(),
        &by,
        "volume.create",
        &name,
        docker.create_volume(&name),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "volume": volume }))))
}

pub async fn remove_volume(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(name): Path<String>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = initiator(&auth, request);
    action_service::run(
        app.db(),
        &by,
        "volume.remove",
        &name,
        docker.remove_volume(&name, scope(&auth)),
    )
    .await?;
    Ok(Json(json!({ "ok": true })))
}

pub async fn networks(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    Ok(Json(
        json!({ "networks": docker(&app)?.networks(scope(&auth)).await? }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewNetwork {
    pub name: String,
    /// `bridge` (one host) or `overlay` (the swarm).
    pub driver: String,
}

pub async fn create_network(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(NewNetwork { name, driver }): Json<NewNetwork>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let mut errors = ValidationErrors::default();
    if !valid_object_name(&name) {
        errors.add("name", "must be 1-128 letters, digits, `_`, `.`, `-`");
    }
    if !matches!(driver.as_str(), "bridge" | "overlay") {
        errors.add("driver", "must be bridge or overlay");
    }
    if !errors.is_empty() {
        return Err(AppError::Validation(errors));
    }
    let docker = docker(&app)?;
    let by = initiator(&auth, request);
    let network = action_service::run(
        app.db(),
        &by,
        "network.create",
        &name,
        docker.create_network(&name, &driver),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(json!({ "network": network }))))
}

pub async fn remove_network(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = initiator(&auth, request);
    action_service::run(
        app.db(),
        &by,
        "network.remove",
        &id,
        docker.remove_network(&id, scope(&auth)),
    )
    .await?;
    Ok(Json(json!({ "ok": true })))
}

/// Docker's rule for volume and network names.
fn valid_object_name(name: &str) -> bool {
    let mut chars = name.chars();
    name.len() <= 128
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// `[registry[:port]/]name[:tag|@digest]`, loosely: Docker has the final say.
fn valid_image_reference(image: &str) -> bool {
    !image.is_empty()
        && image.len() <= 255
        && image
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | ':' | '.' | '_' | '-' | '@'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_names_and_references() {
        assert!(valid_object_name("app_data-1.v2"));
        assert!(!valid_object_name("-x") && !valid_object_name("") && !valid_object_name("a b"));
        assert!(!valid_object_name(&"a".repeat(129)));
        assert!(valid_image_reference("ghcr.io/org/app:1.2@sha256:abc"));
        assert!(!valid_image_reference("nginx; rm -rf /") && !valid_image_reference(""));
    }
}
