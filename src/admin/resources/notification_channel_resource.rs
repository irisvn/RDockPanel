//! Notification channels (Settings → Notifications): Discord, Slack,
//! Telegram, email (SMTP) or a webhook, and the events each one gets. The
//! webhook URL, bot token or SMTP password is sealed, never shown again.

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::DatabaseTransaction;
use serde_json::Value;

use crate::admin::{int, is_set, text};
use crate::models::notification_channel;
use crate::services::notify_service::valid_url;

pub struct NotificationChannelResource;

#[async_trait]
impl Resource for NotificationChannelResource {
    type Entity = notification_channel::Entity;
    const SLUG: &'static str = "notification_channels";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Notification channel")
            .plural("Notification channels")
            .title("name")
            .icon("settings")
            .group("Settings")
            .unique(
                "notification_channels_name_unique",
                &["name"],
                "is already taken",
            )
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(notification_channel::Column::Name)
                .required()
                .sortable()
                .max_length(64),
            Field::column(notification_channel::Column::Kind).required(),
            Field::column(notification_channel::Column::Enabled),
            Field::column(notification_channel::Column::OnDeploy),
            Field::column(notification_channel::Column::OnBackup),
            Field::column(notification_channel::Column::OnDisk),
            Field::column(notification_channel::Column::DiskThreshold),
            Field::column(notification_channel::Column::ChatId).max_length(64),
            Field::column(notification_channel::Column::SmtpHost).max_length(255),
            Field::column(notification_channel::Column::SmtpPort),
            Field::column(notification_channel::Column::SmtpUsername).max_length(255),
            Field::column(notification_channel::Column::SmtpFrom).max_length(255),
            Field::column(notification_channel::Column::EmailTo).max_length(1024),
            Field::column(notification_channel::Column::Secret).encrypted(),
        ]
    }

    fn form() -> Form {
        Form::new().columns(2).schema(components![
            TextInput::make("name"),
            SelectInput::make("kind").options([
                ("discord", "Discord"),
                ("slack", "Slack"),
                ("telegram", "Telegram"),
                ("email", "Email (SMTP)"),
                ("webhook", "Webhook"),
            ]),
            SecretInput::make("secret")
                .column_span(2)
                .help("The webhook URL (Discord, Slack, webhook), the bot token (Telegram) or the SMTP password"),
            TextInput::make("chat_id").help("Telegram: the chat's id"),
            TextInput::make("smtp_host"),
            NumberInput::make("smtp_port"),
            TextInput::make("smtp_username"),
            TextInput::make("smtp_from"),
            TextInput::make("email_to").help("Recipients, comma-separated"),
            Toggle::make("enabled"),
            Toggle::make("on_deploy"),
            Toggle::make("on_backup"),
            Toggle::make("on_disk"),
            NumberInput::make("disk_threshold").help("%"),
        ])
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![TextColumn::make("name"), TextColumn::make("kind")])
            .default_sort("name")
    }

    async fn mutate_before_create(
        ctx: &Ctx,
        _txn: &DatabaseTransaction,
        data: Data,
    ) -> AppResult<Data> {
        check(ctx, &data, None)?;
        Ok(data)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        _txn: &DatabaseTransaction,
        record: &notification_channel::Model,
        data: Data,
    ) -> AppResult<Data> {
        check(ctx, &data, Some(record))?;
        Ok(data)
    }
}

/// The secret a write sets (`{"replace": "…"}`), if any.
fn new_secret(data: &Data) -> Option<&str> {
    match data.get("secret")? {
        Value::Object(o) => o.get("replace")?.as_str(),
        _ => None,
    }
}

fn check(ctx: &Ctx, data: &Data, record: Option<&notification_channel::Model>) -> AppResult<()> {
    let mut errors = ValidationErrors::new();
    let after = |field: &str, old: Option<&str>| -> Option<String> {
        if data.contains_key(field) {
            text(data, field).map(|t| t.trim().to_owned())
        } else {
            old.map(str::to_owned)
        }
    };
    let kind = after("kind", record.map(|r| r.kind.as_str())).unwrap_or_default();
    let secret = new_secret(data);
    let has_secret = secret.is_some() || record.is_some_and(|r| r.secret.is_some());
    match kind.as_str() {
        "discord" | "slack" | "webhook" => {
            if secret.is_some_and(|s| !valid_url(s.trim())) || !has_secret {
                errors.add("secret", ctx.t("must be the webhook's http(s) URL"));
            }
        }
        "telegram" => {
            if !has_secret {
                errors.add("secret", ctx.t("is the bot token (from BotFather)"));
            }
            if !after("chat_id", record.and_then(|r| r.chat_id.as_deref()))
                .is_some_and(|c| !c.is_empty())
            {
                errors.add("chat_id", ctx.t("is required"));
            }
        }
        "email" => {
            for field in ["smtp_host", "smtp_from", "email_to"] {
                let old = record.and_then(|r| match field {
                    "smtp_host" => r.smtp_host.as_deref(),
                    "smtp_from" => r.smtp_from.as_deref(),
                    _ => r.email_to.as_deref(),
                });
                if !after(field, old).is_some_and(|v| !v.is_empty()) {
                    errors.add(field, ctx.t("is required"));
                }
            }
            if is_set(data, "smtp_port")
                && !int(data, "smtp_port").is_some_and(|p| (1..=65535).contains(&p))
            {
                errors.add("smtp_port", ctx.t("must be between 1 and 65535"));
            }
        }
        _ => errors.add(
            "kind",
            ctx.t("must be discord, slack, telegram, email or webhook"),
        ),
    }
    if is_set(data, "disk_threshold")
        && !int(data, "disk_threshold").is_some_and(|p| (1..=100).contains(&p))
    {
        errors.add("disk_threshold", ctx.t("must be between 1 and 100"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}
