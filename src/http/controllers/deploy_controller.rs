//! A deployment's progress, live (`routes/admin.rs`, under
//! `/admin/api/x/deploy`): read from its row every second, so it follows a
//! run whichever process runs it.

use std::time::Duration;

use rustcms::auth::{EventStream, LiveSession};
use rustcms::prelude::*;
use rustcms::realtime::{Event, StreamExt, stream, time};
use rustcms::{AppError, AppResult};
use sea_orm::EntityTrait;
use serde_json::json;

use crate::models::deployment;
use crate::services::deployment_service::summary;

/// `status` events (the deployment without its log) and `log` events (the
/// whole log first, and when the kept tail moved: `{"reset": …}`; else what
/// was added: `{"append": …}`), until the deployment is over.
pub async fn log(
    live: LiveSession,
    State(app): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<EventStream> {
    let db = app.db().clone();
    deployment::Entity::find_by_id(id)
        .one(&db)
        .await?
        .ok_or(AppError::NotFound)?;
    let events = stream::unfold(
        (db, None::<String>, false),
        move |(db, sent, over): (_, Option<String>, bool)| async move {
            if over {
                return None;
            }
            let row = deployment::Entity::find_by_id(id)
                .one(&db)
                .await
                .ok()
                .flatten()?;
            let log = row.log.clone().unwrap_or_default();
            let mut events = Vec::new();
            // Each connection starts over (a reconnect included).
            let change = match sent.as_deref().map(|sent| log.strip_prefix(sent)) {
                Some(Some("")) => None,
                Some(Some(added)) => Some(json!({ "append": added })),
                _ => Some(json!({ "reset": log })),
            };
            if let Some(change) = change {
                events.extend(Event::json("log", &change).ok());
            }
            let mut status = summary(&row);
            status["log"] = serde_json::Value::Null;
            let finished = status["finished"].as_bool().unwrap_or(true);
            events.extend(Event::json("status", &status).ok());
            if !finished {
                time::sleep(Duration::from_secs(1)).await;
            }
            Some((stream::iter(events), (db, Some(log), finished)))
        },
    )
    .flatten();
    Ok(live.stream(events))
}
