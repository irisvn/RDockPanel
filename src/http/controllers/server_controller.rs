//! The server page's endpoints (`routes/admin.rs`, under
//! `/admin/api/x/server`): the host and its recent samples. Pages poll
//! `stats` until the framework streams server-sent events.

use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use serde_json::Value;

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
