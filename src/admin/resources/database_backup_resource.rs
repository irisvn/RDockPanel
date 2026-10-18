//! A database service's backup configurations (its Backups tab): a database,
//! a cron schedule (UTC), a storage provider, a path in it and a retention.
//! Runs are asked for through the apps API (`/apps/backups/{id}/run`).

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::{DatabaseTransaction, EntityTrait};

use crate::admin::{int, is_set, text};
use crate::models::{database_backup, service};
use crate::services::backup_service::{parse_schedule, supported, valid_database};
use crate::services::database_service::Engine;
use crate::services::storage_service::valid_key;

/// The most files a configuration keeps.
const RETENTION: std::ops::RangeInclusive<i64> = 1..=1000;

pub struct DatabaseBackupResource;

#[async_trait]
impl Resource for DatabaseBackupResource {
    type Entity = database_backup::Entity;
    const SLUG: &'static str = "database_backups";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Database backup")
            .plural("Database backups")
            .title("database_name")
            .icon("history")
            .group("Deploy")
            .lock_parent("service_id", "services")
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(database_backup::Column::ServiceId)
                .belongs_to("services")
                .required()
                .filterable(),
            Field::column(database_backup::Column::StorageProviderId)
                .belongs_to("storage_providers")
                .required(),
            Field::column(database_backup::Column::DatabaseName)
                .required()
                .sortable()
                .max_length(64),
            Field::column(database_backup::Column::Enabled),
            Field::column(database_backup::Column::Schedule)
                .required()
                .max_length(64),
            Field::column(database_backup::Column::Retention).required(),
            Field::column(database_backup::Column::Path)
                .required()
                .max_length(400),
            Field::column(database_backup::Column::LastSlotAt)
                .guarded()
                .readable(|_| true),
        ]
    }

    fn form() -> Form {
        Form::new().columns(2).schema(components![
            SelectInput::make("service_id"),
            SelectInput::make("storage_provider_id"),
            TextInput::make("database_name"),
            Toggle::make("enabled").help("Run on the schedule (it can always be run by hand)"),
            TextInput::make("schedule").help("Cron, UTC: 0 2 * * * is daily at 02:00"),
            NumberInput::make("retention").help("Files of this database kept in the path"),
            TextInput::make("path")
                .column_span(2)
                .help("A folder of the provider, e.g. production/orders-db"),
        ])
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("database_name"),
                TextColumn::make("service_id"),
                TextColumn::make("schedule"),
                TextColumn::make("path"),
            ])
            .default_sort("database_name")
    }

    async fn mutate_before_create(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        data: Data,
    ) -> AppResult<Data> {
        check(ctx, txn, &data, None).await?;
        Ok(data)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        record: &database_backup::Model,
        data: Data,
    ) -> AppResult<Data> {
        check(ctx, txn, &data, Some(record)).await?;
        Ok(data)
    }
}

async fn check(
    ctx: &Ctx,
    txn: &DatabaseTransaction,
    data: &Data,
    record: Option<&database_backup::Model>,
) -> AppResult<()> {
    let mut errors = ValidationErrors::new();
    let service_id = int(data, "service_id").or(record.map(|r| r.service_id));
    if let Some(id) = service_id {
        let engine = service::Entity::find_by_id(id)
            .one(txn)
            .await?
            .and_then(|s| Engine::parse(&s.kind));
        if !engine.is_some_and(supported) {
            errors.add(
                "service_id",
                ctx.t("must be a PostgreSQL, MySQL, MariaDB or MongoDB service"),
            );
        }
    }
    if text(data, "database_name").is_some_and(|d| !valid_database(d.trim())) {
        errors.add(
            "database_name",
            ctx.t("must be letters, digits, _, - and $ (not starting with -)"),
        );
    }
    if text(data, "schedule").is_some_and(|s| parse_schedule(s).is_none()) {
        errors.add(
            "schedule",
            ctx.t("is not a five-field cron expression (e.g. 0 2 * * *)"),
        );
    }
    if is_set(data, "retention") && !int(data, "retention").is_some_and(|n| RETENTION.contains(&n))
    {
        errors.add("retention", ctx.t("must be between 1 and 1000"));
    }
    if text(data, "path").is_some_and(|p| !valid_key(p.trim().trim_matches('/'))) {
        errors.add(
            "path",
            ctx.t("must be folders of letters, digits, ., _ and - (e.g. production/orders)"),
        );
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}
