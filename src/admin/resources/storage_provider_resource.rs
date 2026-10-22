//! Storage providers: where database backups go (Settings → Storage
//! providers). A local directory of this server, or an S3-compatible bucket
//! (its secret key sealed, never shown). One in use by a backup cannot be
//! deleted.

use std::sync::Arc;

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::DatabaseTransaction;

use crate::admin::text;
use crate::models::storage_provider;
use crate::policies::storage_provider_policy::StorageProviderPolicy;
use crate::services::storage_service::valid_endpoint;

pub struct StorageProviderResource;

#[async_trait]
impl Resource for StorageProviderResource {
    type Entity = storage_provider::Entity;
    const SLUG: &'static str = "storage_providers";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Storage provider")
            .plural("Storage providers")
            .title("name")
            .icon("settings")
            .group("Settings")
            .navigation(false)
            .unique(
                "storage_providers_name_unique",
                &["name"],
                "is already taken",
            )
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(storage_provider::Column::Name)
                .required()
                .searchable()
                .sortable()
                .max_length(64),
            Field::column(storage_provider::Column::Kind).required(),
            Field::column(storage_provider::Column::LocalPath).max_length(512),
            Field::column(storage_provider::Column::Endpoint).max_length(512),
            Field::column(storage_provider::Column::Region).max_length(64),
            Field::column(storage_provider::Column::Bucket).max_length(255),
            Field::column(storage_provider::Column::AccessKey).max_length(255),
            Field::column(storage_provider::Column::SecretKey).encrypted(),
            Field::column(storage_provider::Column::PathStyle),
            Field::column(storage_provider::Column::CreatedAt)
                .guarded()
                .sortable(),
        ]
    }

    fn form() -> Form {
        Form::new().columns(2).schema(components![
            TextInput::make("name"),
            SelectInput::make("kind")
                .options([("local", "Local directory"), ("s3", "S3-compatible")]),
            TextInput::make("local_path")
                .column_span(2)
                .help("Local: an absolute directory of the server, e.g. /var/backups/rdockpanel"),
            TextInput::make("endpoint")
                .column_span(2)
                .help("S3: e.g. https://s3.eu-west-1.amazonaws.com"),
            TextInput::make("region"),
            TextInput::make("bucket"),
            TextInput::make("access_key"),
            SecretInput::make("secret_key"),
            Toggle::make("path_style")
                .help("Path-style URLs (MinIO and most S3-compatible services)"),
        ])
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("name"),
                TextColumn::make("kind"),
                TextColumn::make("bucket"),
                TextColumn::make("created_at"),
            ])
            .default_sort("name")
    }

    fn policy() -> Arc<dyn Policy> {
        Arc::new(StorageProviderPolicy)
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
        record: &storage_provider::Model,
        data: Data,
    ) -> AppResult<Data> {
        check(ctx, &data, Some(record))?;
        Ok(data)
    }
}

fn check(ctx: &Ctx, data: &Data, record: Option<&storage_provider::Model>) -> AppResult<()> {
    let mut errors = ValidationErrors::new();
    // The values after this write.
    let after = |field: &str, old: Option<&str>| -> Option<String> {
        if data.contains_key(field) {
            text(data, field).map(|t| t.trim().to_owned())
        } else {
            old.map(str::to_owned)
        }
    };
    let kind = after("kind", record.map(|r| r.kind.as_str())).unwrap_or_default();
    match kind.as_str() {
        "local" => {
            let path = after("local_path", record.and_then(|r| r.local_path.as_deref()))
                .unwrap_or_default();
            let path = std::path::Path::new(&path);
            if !path.is_absolute()
                || path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                || path == std::path::Path::new("/")
            {
                errors.add("local_path", ctx.t("must be an absolute directory (not /)"));
            }
        }
        "s3" => {
            if !after("endpoint", record.and_then(|r| r.endpoint.as_deref()))
                .is_some_and(|e| valid_endpoint(&e))
            {
                errors.add("endpoint", ctx.t("must be an http(s) URL"));
            }
            if !after("bucket", record.and_then(|r| r.bucket.as_deref()))
                .is_some_and(|b| !b.is_empty() && b.len() <= 63 && !b.contains('/'))
            {
                errors.add("bucket", ctx.t("is required"));
            }
            if !after("access_key", record.and_then(|r| r.access_key.as_deref()))
                .is_some_and(|k| !k.is_empty())
            {
                errors.add("access_key", ctx.t("is required"));
            }
        }
        _ => errors.add("kind", ctx.t("must be local or s3")),
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}
