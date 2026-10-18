//! The server page's endpoints (`routes/admin.rs`, under
//! `/admin/api/x/server`): the host, its recent samples, and the samples as
//! they come (server-sent events).

use rustcms::auth::{EventStream, LiveSession};
use rustcms::prelude::*;
use rustcms::realtime::{Event, StreamExt, stream};
use rustcms::{AppError, AppResult, Json};
use serde_json::Value;

use crate::services::live_service::Live;
use crate::services::system_stats_service::{INTERVAL, SystemStats};

fn stats(app: &AppState) -> AppResult<std::sync::Arc<SystemStats>> {
    app.service::<SystemStats>()
        .ok_or_else(|| AppError::internal("SystemStats is not bound"))
}

pub async fn info(State(app): State<AppState>) -> AppResult<Json<Value>> {
    Ok(Json(json!(stats(&app)?.info())))
}

/// The samples kept (about five minutes), oldest first.
pub async fn samples(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let stats = stats(&app)?;
    let samples = stats.history();
    Ok(Json(
        json!({ "interval_ms": INTERVAL.as_millis() as u64, "samples": samples }),
    ))
}

/// `history` (the samples kept) first, then each new `sample`.
pub async fn live(live: LiveSession, State(app): State<AppState>) -> AppResult<EventStream> {
    let stats = stats(&app)?;
    let hub = app
        .service::<Live>()
        .ok_or_else(|| AppError::internal("Live is not bound"))?;
    let samples = hub.server((*stats).clone())?;
    let history = json!({ "interval_ms": INTERVAL.as_millis() as u64, "samples": stats.history() });
    let first = stream::iter(Event::json("history", &history).ok());
    Ok(live.stream(first.chain(samples.into_events())))
}
