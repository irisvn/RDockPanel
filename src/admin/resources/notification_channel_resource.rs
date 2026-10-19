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
            .navigation(false)
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
            Field::column(notification_channel::Column::Events).max_length(1000),
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
        Ok(in_step(data, None))
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        _txn: &DatabaseTransaction,
        record: &notification_channel::Model,
        data: Data,
    ) -> AppResult<Data> {
        check(ctx, &data, Some(record))?;
        Ok(in_step(data, Some(record)))
    }
}

/// `events` and the older `on_deploy`, `on_backup`, `on_disk` kept in step,
/// whichever a write sets: the events decide what a channel hears.
fn in_step(mut data: Data, record: Option<&notification_channel::Model>) -> Data {
    const FLAGS: [(&str, &str); 3] = [
        ("deploy", "on_deploy"),
        ("backup", "on_backup"),
        ("disk", "on_disk"),
    ];
    if let Some(list) =
        text(&data, "events").and_then(|e| serde_json::from_str::<Vec<String>>(e).ok())
    {
        for (event, flag) in FLAGS {
            data.insert(
                flag.to_owned(),
                Value::Bool(list.iter().any(|e| e == event)),
            );
        }
        return data;
    }
    let set: Vec<(&str, bool)> = FLAGS
        .iter()
        .filter_map(|(event, flag)| {
            data.get(*flag)
                .and_then(Value::as_bool)
                .map(|on| (*event, on))
        })
        .collect();
    if set.is_empty() {
        return data;
    }
    // Its events now (when it has some), each flag set applied.
    if let Some(record) = record.filter(|r| r.events.is_some()) {
        let mut list = crate::services::notify_service::events_of(record);
        for (event, on) in set {
            list.retain(|e| e != event);
            if on {
                list.push(event.to_owned());
            }
        }
        data.insert(
            "events".to_owned(),
            Value::String(serde_json::to_string(&list).expect("names serialize")),
        );
    }
    data
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
    // The events it hears: a JSON array of known names.
    if let Some(events) = text(data, "events") {
        let known = |e: &String| {
            crate::services::notify_service::EVENTS
                .iter()
                .any(|(name, _)| name == e)
        };
        if !serde_json::from_str::<Vec<String>>(events).is_ok_and(|list| list.iter().all(known)) {
            errors.add(
                "events",
                ctx.t("must be a list of deploy, backup, disk, certificate, service, traefik, update, prune, login"),
            );
        }
    }
    let secret = new_secret(data);
    // A secret goes where it was meant to: another kind of channel, or
    // another SMTP server, needs it entered again (never the old one sent
    // elsewhere).
    let moved = record.is_some_and(|r| {
        let changed = |field: &str, old: Option<String>| {
            data.contains_key(field)
                && data.get(field).map(|v| match v {
                    Value::Null => None,
                    Value::String(s) => Some(s.trim().to_owned()),
                    other => Some(other.to_string()),
                }) != Some(old)
        };
        r.kind != kind
            || changed("smtp_host", r.smtp_host.clone())
            || changed("smtp_port", r.smtp_port.map(|p| p.to_string()))
            || changed("smtp_username", r.smtp_username.clone())
    });
    if moved && secret.is_none() && record.is_some_and(|r| r.secret.is_some()) {
        errors.add(
            "secret",
            ctx.t("enter it again: the channel's kind or server changed"),
        );
    }
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
