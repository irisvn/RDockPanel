//! Settings → Panel's controls (`/admin/api/x/apps/system/...`), like
//! EasyPanel's: the panel's logs, restart and update check, the server's
//! address and reboot, Docker's prunes. `system.manage` (the routes).

use futures_util::StreamExt;
use rustcms::audit::RequestMeta;
use rustcms::auth::{EventStream, LiveSession};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use serde_json::Value;

use crate::services::action_service::{self, Initiator};
use crate::services::docker_service::{Docker, LogLine, LogQuery, LogStream, PruneKind};
use crate::services::system_service::{self, SELF_SERVICE};

/// Installed by install.sh (`[panel] installed`).
fn installed() -> bool {
    crate::services::deploy_rules_service::config().installed
}

fn docker(app: &AppState) -> AppResult<std::sync::Arc<Docker>> {
    app.service::<Docker>()
        .ok_or_else(|| AppError::internal("Docker is not configured"))
}

pub async fn status(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let docker = app.service::<Docker>();
    Ok(Json(
        system_service::status(&app, docker.as_deref(), installed()).await,
    ))
}

/// The panel's log, live: its service's (installed), else the lines this
/// process kept (from source).
pub async fn logs(
    live: LiveSession,
    State(app): State<AppState>,
    Path(tail): Path<u32>,
) -> AppResult<EventStream> {
    let tail = tail.min(crate::services::panel_log::KEPT as u32);
    let docker = docker(&app)?;
    let status = system_service::status(&app, Some(&docker), installed()).await;
    if status["service"] == true {
        let query = LogQuery {
            follow: true,
            tail: Some(tail),
            since: None,
            timestamps: false,
        };
        let lines = docker.engine().service_logs(SELF_SERVICE, query);
        return Ok(live.stream(crate::services::live_service::log_events(lines)));
    }
    let (kept, follow) = crate::services::panel_log::follow(tail as usize);
    let line = |text: String| {
        Ok(LogLine {
            stream: LogStream::Stdout,
            text,
        })
    };
    let lines =
        futures_util::stream::iter(kept.into_iter().map(line)).chain(followed(follow).map(line));
    Ok(live.stream(crate::services::live_service::log_events(lines)))
}

/// The lines written from now on (a follower too slow skips what it missed).
fn followed(
    rx: tokio::sync::broadcast::Receiver<String>,
) -> impl futures_util::Stream<Item = String> + Send + 'static {
    use tokio::sync::broadcast::error::RecvError;
    futures_util::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(line) => return Some((line, rx)),
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    })
}

pub async fn restart(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    system_service::restart(&app, &docker, &by, installed()).await?;
    Ok(Json(json!({ "restarting": true })))
}

#[derive(Debug, serde::Deserialize)]
pub struct Reboot {
    #[serde(default)]
    pub confirm: String,
}

pub async fn reboot(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Reboot>,
) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    system_service::reboot(&app, &docker, &by, &body.confirm, installed()).await?;
    Ok(Json(json!({ "rebooting": true })))
}

pub async fn check_update(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let docker = docker(&app)?;
    Ok(Json(
        system_service::check_update(&docker, installed()).await?,
    ))
}

/// Finds the server's public address again (logged as an action).
pub async fn refresh_ip(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
) -> AppResult<Json<Value>> {
    let by = Initiator::user(auth.principal(), request);
    let found = action_service::run(
        app.db(),
        &by,
        "server.refresh_ip",
        "public_ip",
        system_service::refresh_ip(app.db()),
    )
    .await?;
    Ok(Json(found))
}

/// Prunes one kind (logged as an action, audited).
pub async fn prune(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(kind): Path<String>,
) -> AppResult<Json<Value>> {
    let kind: PruneKind = serde_json::from_value(json!(kind)).map_err(|_| AppError::NotFound)?;
    let docker = docker(&app)?;
    let by = Initiator::user(auth.principal(), request);
    let name = serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let report = action_service::run(
        app.db(),
        &by,
        &format!("system.prune.{name}"),
        "docker",
        docker.prune_kind(kind),
    )
    .await?;
    crate::services::notify_service::notify(
        &app,
        crate::services::notify_service::Event::Prune {
            kind: name.replace('_', " "),
            removed: report.containers_deleted
                + report.images_deleted
                + report.volumes_deleted
                + report.build_caches_deleted,
            reclaimed: report.space_reclaimed,
        },
    );
    Ok(Json(json!({ "report": report })))
}
