//! A service's scripts (its Scripts tab): a name and a shell command. Running
//! one is `/apps/scripts/{id}/run` (it needs `docker.exec`, as a terminal).

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::DatabaseTransaction;

use crate::admin::text;
use crate::models::service_script;

pub struct ServiceScriptResource;

#[async_trait]
impl Resource for ServiceScriptResource {
    type Entity = service_script::Entity;
    const SLUG: &'static str = "service_scripts";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Script")
            .plural("Scripts")
            .title("name")
            .icon("box")
            .group("Deploy")
            .navigation(false)
            .lock_parent("service_id", "services")
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(service_script::Column::ServiceId)
                .belongs_to("services")
                .required()
                .filterable(),
            Field::column(service_script::Column::Name)
                .required()
                .sortable()
                .max_length(64),
            Field::column(service_script::Column::Command)
                .required()
                .max_length(10_000),
        ]
    }

    fn form() -> Form {
        Form::new().schema(components![
            SelectInput::make("service_id"),
            TextInput::make("name").help("e.g. Migrate"),
            Textarea::make("command")
                .help("Run with sh -c in a running container, e.g. php artisan migrate --force"),
        ])
    }

    fn policy() -> std::sync::Arc<dyn Policy> {
        std::sync::Arc::new(crate::admin::resources::guard::ChildPolicy(
            "service_scripts",
        ))
    }

    /// A member sees their projects' records only (a role seeing every
    /// record: all of them).
    fn scope(
        query: sea_orm::Select<<Self as Resource>::Entity>,
        ctx: &Ctx,
    ) -> sea_orm::Select<<Self as Resource>::Entity> {
        use sea_orm::{ColumnTrait, QueryFilter};
        match crate::admin::resources::guard::members_only(ctx, "service_scripts") {
            Some(user) => query.filter(
                crate::models::service_script::Column::ServiceId
                    .in_subquery(crate::services::access_service::member_services(user)),
            ),
            None => query,
        }
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("name"),
                TextColumn::make("service_id"),
            ])
            .default_sort("name")
    }

    async fn mutate_before_create(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        data: Data,
    ) -> AppResult<Data> {
        crate::admin::resources::guard::write(ctx, txn, &data, None, "service_scripts").await?;
        check(ctx, &data)?;
        Ok(data)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        record: &service_script::Model,
        data: Data,
    ) -> AppResult<Data> {
        crate::admin::resources::guard::write(
            ctx,
            txn,
            &data,
            Some(record.service_id),
            "service_scripts",
        )
        .await?;
        check(ctx, &data)?;
        Ok(data)
    }
}

fn check(ctx: &Ctx, data: &Data) -> AppResult<()> {
    let mut errors = ValidationErrors::new();
    if text(data, "name").is_some_and(|n| n.trim().is_empty()) {
        errors.add("name", ctx.t("is required"));
    }
    if text(data, "command").is_some_and(|c| c.trim().is_empty()) {
        errors.add("command", ctx.t("is required"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.into())
    }
}
