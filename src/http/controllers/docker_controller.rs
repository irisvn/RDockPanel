//! The Docker pages' endpoints (`routes/admin.rs`, under
//! `/admin/api/x/docker`). Each route's permission is checked before these
//! run; here, the panel's own objects need `system.manage` on top
//! ([`scope`]), and every change is written to the actions log.

use futures_util::StreamExt;
use rustcms::Principal;
use rustcms::audit::{self, AuditEntry, RequestMeta};
use rustcms::auth::{
    AdminSocket, EventStream, LiveSession, LiveSocket, SocketMessage, SocketResponse, TargetFuture,
};
use rustcms::prelude::*;
use rustcms::queue;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use serde::Deserialize;
use serde_json::Value;

use crate::jobs::pull_image::PullImage;
use crate::models::action;
use crate::policies::permissions;
use crate::services::action_service::{self, Initiator};
use crate::services::docker_service::{
    ContainerAction, Docker, LogQuery, Scope, TermSize, session_token,
};
use crate::services::live_service::Live;

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
    scope_of(auth.principal())
}

pub fn scope_of(principal: &Principal) -> Scope {
    if principal.can(permissions::SYSTEM_MANAGE) {
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
    initiator_of(auth.principal(), request)
}

fn initiator_of(principal: &Principal, request: RequestMeta) -> Initiator {
    Initiator::user(principal, request)
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

/// The container's output as it comes (`lines` events): the last `tail`
/// lines first (again after a reconnect: the page starts over on `open`).
pub async fn follow_logs(
    live: LiveSession,
    State(app): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<LogParams>,
) -> AppResult<EventStream> {
    follow(live, app, id, params).await
}

/// [`follow_logs`] with the tail in the path (`…/logs/follow/{tail}`): the
/// panel's event streams take no query string (RustCMS 0.5).
pub async fn follow_logs_tail(
    live: LiveSession,
    State(app): State<AppState>,
    Path((id, tail)): Path<(String, u32)>,
) -> AppResult<EventStream> {
    let params = LogParams {
        tail: Some(tail),
        since: None,
        timestamps: false,
    };
    follow(live, app, id, params).await
}

async fn follow(
    live: LiveSession,
    app: AppState,
    id: String,
    params: LogParams,
) -> AppResult<EventStream> {
    let docker = docker(&app)?;
    let id = docker.container_id(&id, scope_of(live.principal())).await?;
    let tail = params.tail.unwrap_or(500).min(MAX_LOG_LINES);
    let query = LogQuery {
        follow: true,
        tail: Some(tail),
        since: params.since,
        timestamps: params.timestamps,
    };
    let lines = docker.logs(&id, query, Scope::System).await?;
    let events = crate::services::live_service::log_events(lines);
    Ok(live.stream(events).target(container_target(docker, id)))
}

/// The container's resources about every second (`stats` events), one
/// Docker stream for all its viewers.
pub async fn live_stats(
    live: LiveSession,
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<EventStream> {
    let docker = docker(&app)?;
    let id = docker.container_id(&id, scope_of(live.principal())).await?;
    let hub = app
        .service::<Live>()
        .ok_or_else(|| AppError::internal("Live is not bound"))?;
    let stats = hub.container_stats((*docker).clone(), id.clone())?;
    Ok(live
        .stream(stats.into_events())
        .target(container_target(docker, id)))
}

/// The terminal's opening size: `?cols=&rows=` (80×24 by default).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TermParams {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
}

/// The log of a running terminal's action: this, then the session's mark.
pub const TERMINAL_SESSION: &str = "session ";

/// A terminal goes after this long without input or output.
const TERMINAL_IDLE: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// A shell in the container, over a WebSocket (`docker.exec`; the panel's own
/// containers also need `system.manage`). Binary messages are the terminal's
/// bytes both ways; text messages are commands:
/// `{"type":"resize","cols":…,"rows":…}`, authorized when they run. The
/// session is logged as a `container.exec` action and audited (never what
/// was typed).
pub async fn exec(
    live: LiveSocket,
    State(app): State<AppState>,
    request: RequestMeta,
    Path(id): Path<String>,
    Query(params): Query<TermParams>,
) -> AppResult<SocketResponse> {
    let docker = docker(&app)?;
    let id = docker.container_id(&id, scope_of(live.principal())).await?;
    let by = initiator_of(live.principal(), request);
    let size = TermSize {
        cols: params.cols.unwrap_or(80),
        rows: params.rows.unwrap_or(24),
    };
    let (db, shell, target) = (
        app.db().clone(),
        docker.clone(),
        container_target(docker, id.clone()),
    );
    Ok(live
        .accept(move |mut socket| async move {
            // Ends the action when the socket does, however it ends (the
            // session's future may be dropped at any await).
            let token = session_token();
            let mut log = ExecLog::start(db, &by, &id, &token).await;
            log.audit(&by, &id).await;
            let allowed = |principal: &Principal| principal.can(permissions::DOCKER_EXEC);
            log.error = terminal(&shell, &id, size, &token, &mut socket, &allowed)
                .await
                .err();
        })
        .target(target))
}

/// Pumps the shell's bytes and the socket's messages until either ends.
async fn terminal(
    docker: &Docker,
    id: &str,
    size: TermSize,
    token: &str,
    socket: &mut AdminSocket,
    // Whether a command (a resize) may still run, on a fresh principal.
    allowed: &(dyn Fn(&Principal) -> bool + Sync),
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    // Dropped with this future, however the socket ends: the shell ends too.
    let mut shell = match docker.exec(id, size, Scope::System, token).await {
        Ok(shell) => shell,
        Err(err) => {
            // Said in the terminal (an image without a shell, …), then closed.
            let message = err.to_string();
            let _ = socket
                .send(SocketMessage::Text(
                    json!({ "type": "error", "message": message }).to_string(),
                ))
                .await;
            return Err(message);
        }
    };
    let idle = tokio::time::sleep(TERMINAL_IDLE);
    tokio::pin!(idle);
    loop {
        tokio::select! {
            message = socket.recv() => match message {
                Some(SocketMessage::Binary(keys)) => {
                    shell.session.input.write_all(&keys).await.map_err(|err| err.to_string())?;
                    shell.session.input.flush().await.map_err(|err| err.to_string())?;
                }
                Some(SocketMessage::Text(command)) => {
                    if let Ok(TermCommand::Resize(size)) = serde_json::from_str(&command) {
                        // A command: checked again now, on a fresh principal.
                        let Ok(principal) = socket.authorize().await else { return Ok(()) };
                        if !allowed(&principal) {
                            return Ok(());
                        }
                        let _ = docker.resize_exec(&shell.session.id, size).await;
                    }
                }
                None => return Ok(()),
            },
            output = shell.session.output.next() => match output {
                Some(Ok(bytes)) => {
                    // Messages stay under the framework's 64 KiB.
                    for chunk in bytes.chunks(32 * 1024) {
                        if socket.send(SocketMessage::Binary(chunk.to_vec())).await.is_err() {
                            return Ok(());
                        }
                    }
                }
                Some(Err(err)) => return Err(err.to_string()),
                None => {
                    let _ = socket.send(SocketMessage::Text(json!({ "type": "exit" }).to_string())).await;
                    return Ok(());
                }
            },
            () = &mut idle => {
                let _ = socket.send(SocketMessage::Text(json!({ "type": "idle" }).to_string())).await;
                return Ok(());
            }
        }
        idle.as_mut()
            .reset(tokio::time::Instant::now() + TERMINAL_IDLE);
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TermCommand {
    Resize(TermSize),
}

/// The `container.exec` action of a terminal: started with it, finished
/// (with how long it lasted) when it is dropped.
struct ExecLog {
    db: sea_orm::DatabaseConnection,
    row: Option<action::Model>,
    started: std::time::Instant,
    error: Option<String>,
}

impl ExecLog {
    /// The action row, guarded at once: from here on, however this ends, the
    /// row ends too. Its log names the session's mark until then, so a panel
    /// stopped meanwhile ends the shell at its next boot.
    async fn start(db: sea_orm::DatabaseConnection, by: &Initiator, id: &str, token: &str) -> Self {
        let row = action_service::start(&db, by, "container.exec", id, action::RUNNING)
            .await
            .ok();
        let row = match row {
            Some(row) => action_service::note(&db, row, &format!("{TERMINAL_SESSION}{token}"))
                .await
                .ok(),
            None => None,
        };
        Self {
            db,
            row,
            started: std::time::Instant::now(),
            error: None,
        }
    }

    async fn audit(&self, by: &Initiator, id: &str) {
        let entry = AuditEntry::new("docker.container.exec", "docker").key(id);
        audit::record_detached(&self.db, &by.audit, entry).await;
    }
}

impl Drop for ExecLog {
    fn drop(&mut self) {
        let (Some(row), Ok(runtime)) = (self.row.take(), tokio::runtime::Handle::try_current())
        else {
            return;
        };
        let (db, error) = (self.db.clone(), self.error.take());
        let log = format!("session lasted {}s", self.started.elapsed().as_secs());
        runtime.spawn(async move {
            if let Err(err) = action_service::finish(&db, row, error, Some(&log)).await {
                tracing::warn!(%err, "could not finish a terminal's action");
            }
        });
    }
}

/// Ends a container's stream once its viewer may no longer see it (the
/// panel's own: `system.manage`) or it is gone.
fn container_target(
    docker: std::sync::Arc<Docker>,
    id: String,
) -> impl Fn(Principal) -> TargetFuture + Send + Sync + 'static {
    move |principal| {
        let (docker, id) = (docker.clone(), id.clone());
        Box::pin(async move {
            docker
                .container_id(&id, scope_of(&principal))
                .await
                .map(drop)
                .map_err(AppError::from)
        })
    }
}

/// The images, each with its containers running and stopped (`null`:
/// unknown).
pub async fn images(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let images: Vec<Value> = docker(&app)?
        .images_with_use()
        .await?
        .into_iter()
        .map(|(image, usage)| {
            let mut value = serde_json::to_value(&image).unwrap_or_default();
            value["running"] = json!(usage.map(|(running, _)| running));
            value["stopped"] = json!(usage.map(|(_, stopped)| stopped));
            value
        })
        .collect();
    Ok(Json(json!({ "images": images })))
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

/// An image named in the query (`?image=nginx:1.27&force=true`): a tag or a
/// registry path does not fit the panel's path segments (`:` and `/`).
#[derive(Debug, Deserialize)]
pub struct ImageRef {
    pub image: String,
    #[serde(default)]
    pub force: bool,
}

/// `DELETE /docker/images?image=…`: [`remove_image`] by a name of any shape.
pub async fn remove_image_named(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Query(ImageRef { image, force }): Query<ImageRef>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = initiator(&auth, request);
    action_service::run(
        app.db(),
        &by,
        "image.remove",
        &image,
        docker.remove_image(&image, force, scope(&auth)),
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
    crate::services::notify_service::notify(
        &app,
        crate::services::notify_service::Event::Prune {
            kind: "containers and dangling images".to_owned(),
            removed: report.containers_deleted + report.images_deleted,
            reclaimed: report.space_reclaimed,
        },
    );
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

/// The permissions a service's logs, and its terminal, need: the service's
/// own (a role's, or its project role's), or Docker's.
const SERVICE_LOGS: &[&str] = &[permissions::SERVICES_LOGS, permissions::DOCKER_LOGS];
const SERVICE_EXEC: &[&str] = &[permissions::SERVICES_EXEC, permissions::DOCKER_EXEC];

/// The service `id`, for `permissions`, as the panel deployed it: its Swarm
/// service's name, only when that service is the panel's own deployment of
/// it (labelled with its project and service), never another object of
/// that name. A Compose or privileged service: its administrators' only.
async fn deployed_service(
    app: &AppState,
    principal: &Principal,
    id: i64,
    permissions: &[&str],
) -> AppResult<String> {
    let row =
        crate::services::access_service::service_any(app.db(), principal, id, permissions).await?;
    crate::http::controllers::apps_controller::forbid_privileged_of(principal, &row)?;
    if row.kind == "compose" {
        return Err(AppError::BadRequest(
            "a Compose service's logs are its containers' (Docker → Containers)".into(),
        ));
    }
    let snapshot =
        crate::services::deployment_service::snapshot_without_env(app.db(), &row).await?;
    let name = snapshot.service_name();
    docker(app)?
        .managed_service(&name, &snapshot.project, &snapshot.service)
        .await
        .map_err(AppError::from)?
        .ok_or(AppError::NotFound)?;
    Ok(name)
}

/// A service's logs, live (every task's: Swarm's service logs), for whoever
/// may see them in its project: no access to Docker's pages needed.
pub async fn service_logs(
    live: LiveSession,
    State(app): State<AppState>,
    Path((id, tail)): Path<(i64, u32)>,
) -> AppResult<EventStream> {
    let name = deployed_service(&app, live.principal(), id, SERVICE_LOGS).await?;
    let docker = docker(&app)?;
    let query = LogQuery {
        follow: true,
        tail: Some(tail.min(MAX_LOG_LINES)),
        since: None,
        timestamps: false,
    };
    let lines = docker.engine().service_logs(&name, query);
    // Again at each recheck: membership, privilege, the deployment.
    let target = move |principal: Principal| {
        let app = app.clone();
        Box::pin(async move {
            deployed_service(&app, &principal, id, SERVICE_LOGS)
                .await
                .map(drop)
        }) as TargetFuture
    };
    Ok(live
        .stream(crate::services::live_service::log_events(lines))
        .target(target))
}

/// The running task container `container` (its full id, exactly) of the
/// Swarm service `name`.
async fn task_container(docker: &Docker, name: &str, container: &str) -> AppResult<()> {
    let tasks = docker
        .engine()
        .list_tasks(name)
        .await
        .map_err(AppError::from)?;
    let found = tasks.iter().any(|t| {
        t.desired_state == Some(crate::services::docker_service::TaskState::RUNNING)
            && t.status
                .as_ref()
                .and_then(|s| s.container_status.as_ref())
                .and_then(|c| c.container_id.as_deref())
                == Some(container)
    });
    if found {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}

/// A shell in one of a service's containers (as [`exec`]), for whoever may
/// run its terminal in its project: the container must be a running task
/// of the panel's deployment of that service, checked again (with the
/// service's privilege) at each recheck.
pub async fn service_exec(
    live: LiveSocket,
    State(app): State<AppState>,
    request: RequestMeta,
    Path((id, container)): Path<(i64, String)>,
) -> AppResult<SocketResponse> {
    let name = deployed_service(&app, live.principal(), id, SERVICE_EXEC).await?;
    let engine = docker(&app)?;
    task_container(&engine, &name, &container).await?;
    let by = initiator_of(live.principal(), request);
    // Its size comes as a command once open (sockets take no query string).
    let size = TermSize { cols: 80, rows: 24 };
    let (db, shell) = (app.db().clone(), engine.clone());
    let target = {
        let (app, container) = (app.clone(), container.clone());
        move |principal: Principal| {
            let (app, container) = (app.clone(), container.clone());
            Box::pin(async move {
                let name = deployed_service(&app, &principal, id, SERVICE_EXEC).await?;
                let engine = docker(&app)?;
                task_container(&engine, &name, &container).await
            }) as TargetFuture
        }
    };
    Ok(live
        .accept(move |mut socket| async move {
            let token = session_token();
            let mut log = ExecLog::start(db, &by, &container, &token).await;
            log.audit(&by, &container).await;
            // Commands run while the terminal lasts (the recheck closes it
            // when the caller may no longer).
            let allowed = |_: &Principal| true;
            log.error = terminal(&shell, &container, size, &token, &mut socket, &allowed)
                .await
                .err();
        })
        .target(target))
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
