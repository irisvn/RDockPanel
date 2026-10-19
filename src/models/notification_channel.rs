//! `notification_channels` table: where the panel sends what happened
//! (deployments, backups, a full disk). Discord, Slack and webhooks by URL,
//! Telegram by bot token and chat, email by SMTP. The URL, token or SMTP
//! password is `secret`, sealed with `APP_KEY` (context
//! `notification_channels.secret`).

use rustcms::crypto::{Context, Encrypted};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "notification_channels")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    /// `discord`, `slack`, `telegram`, `email` or `webhook`.
    pub kind: String,
    pub enabled: bool,
    /// A deployment that succeeded (Deploy, Rollback) or failed.
    pub on_deploy: bool,
    /// A backup or restore that finished.
    pub on_backup: bool,
    /// The server's disk used at `disk_threshold` % or more.
    pub on_disk: bool,
    pub disk_threshold: i32,
    /// The events it hears by name (a JSON array of [`EVENTS`]' names);
    /// none: its `on_*` choices and the panel's alerts, as before.
    ///
    /// [`EVENTS`]: crate::services::notify_service::EVENTS
    #[sea_orm(column_type = "Text", nullable)]
    pub events: Option<String>,
    /// Telegram: the chat (`-100…` for a group).
    pub chat_id: Option<String>,
    pub smtp_host: Option<String>,
    pub smtp_port: Option<i32>,
    pub smtp_username: Option<String>,
    pub smtp_from: Option<String>,
    /// Recipients, comma-separated.
    pub email_to: Option<String>,
    /// The webhook URL, the bot token, or the SMTP password.
    #[sea_orm(column_type = "Text", nullable)]
    pub secret: Option<Encrypted<String>>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Where `secret` is sealed.
pub fn secret_context() -> Context {
    Context::column("notification_channels", "secret")
}
