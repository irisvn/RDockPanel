//! Notification channels in Settings (`/admin/api/x/apps`): the list (never
//! a secret: only whether one is set) and "Send test notification". The
//! channels are written through the `notification_channels` resource.

use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use sea_orm::{EntityTrait, QueryOrder};
use serde_json::Value;

use crate::models::notification_channel;
use crate::services::notify_service::{self, Event};

pub async fn index(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let rows = notification_channel::Entity::find()
        .order_by_asc(notification_channel::Column::Name)
        .all(app.db())
        .await?;
    Ok(Json(json!({
        "channels": rows.iter().map(|c| json!({
            "id": c.id,
            "name": c.name,
            "kind": c.kind,
            "enabled": c.enabled,
            "on_deploy": c.on_deploy,
            "on_backup": c.on_backup,
            "on_disk": c.on_disk,
            "disk_threshold": c.disk_threshold,
            "chat_id": c.chat_id,
            "smtp_host": c.smtp_host,
            "smtp_port": c.smtp_port,
            "smtp_username": c.smtp_username,
            "smtp_from": c.smtp_from,
            "email_to": c.email_to,
            "secret_set": c.secret.is_some(),
        })).collect::<Vec<_>>(),
    })))
}

/// Sends a test notification now: `{"ok","error"?}`.
pub async fn test(State(app): State<AppState>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let channel = notification_channel::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(
        match notify_service::send(&app, &channel, &Event::Test).await {
            Ok(()) => json!({ "ok": true }),
            Err(error) => json!({ "ok": false, "error": error }),
        },
    ))
}
