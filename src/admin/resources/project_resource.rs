//! Projects: groups of services sharing a network. A project's name is part
//! of its services' Docker objects: it is fixed once one was deployed, and a
//! project with services cannot be deleted.

use std::sync::Arc;

use rustcms::admin::*;
use rustcms::{AppResult, ValidationErrors, async_trait};
use sea_orm::{
    ColumnTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect,
};

use crate::admin::text;
use crate::models::{project, service};
use crate::policies::project_policy::ProjectPolicy;
use crate::services::deploy_rules_service::valid_name;

pub struct ProjectResource;

#[async_trait]
impl Resource for ProjectResource {
    type Entity = project::Entity;
    const SLUG: &'static str = "projects";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Project")
            .title("name")
            .icon("folder")
            .group("Deploy")
            .navigation(false)
            .unique("projects_name_unique", &["name"], "is already taken")
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(project::Column::Name)
                .required()
                .searchable()
                .sortable()
                .max_length(40),
            Field::column(project::Column::Description).max_length(2000),
            Field::column(project::Column::CreatedAt)
                .guarded()
                .sortable(),
        ]
    }

    fn form() -> Form {
        Form::new().schema(components![
            TextInput::make("name").help("Lowercase letters, digits and dashes, e.g. shop"),
            Textarea::make("description"),
        ])
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("name"),
                TextColumn::make("description"),
                TextColumn::make("created_at"),
            ])
            .default_sort("name")
    }

    fn relations() -> Vec<Relation> {
        vec![Relation::has_many("services", "services", "project_id").label("Services")]
    }

    fn policy() -> Arc<dyn Policy> {
        Arc::new(ProjectPolicy)
    }

    /// A member sees their projects' records only (a role seeing every
    /// record: all of them).
    fn scope(
        query: sea_orm::Select<<Self as Resource>::Entity>,
        ctx: &Ctx,
    ) -> sea_orm::Select<<Self as Resource>::Entity> {
        use sea_orm::{ColumnTrait, QueryFilter};
        match crate::policies::members_only(ctx, "projects") {
            Some(user) => query.filter(
                crate::models::project::Column::Id
                    .in_subquery(crate::services::access_service::member_projects(user)),
            ),
            None => query,
        }
    }

    async fn mutate_before_create(
        ctx: &Ctx,
        _txn: &DatabaseTransaction,
        data: Data,
    ) -> AppResult<Data> {
        check_name(ctx, &data)?;
        Ok(data)
    }

    async fn mutate_before_update(
        ctx: &Ctx,
        txn: &DatabaseTransaction,
        record: &project::Model,
        data: Data,
    ) -> AppResult<Data> {
        check_name(ctx, &data)?;
        // Locked: deployments asked for meanwhile (they lock it shared) wait.
        project::Entity::find_by_id(record.id)
            .lock_exclusive()
            .one(txn)
            .await?;
        if text(&data, "name").is_some_and(|name| name != record.name)
            && deployed_services(txn, record.id).await? > 0
        {
            return Err(ValidationErrors::field(
                "name",
                ctx.t("cannot change once a service was deployed"),
            )
            .into());
        }
        Ok(data)
    }
}

fn check_name(ctx: &Ctx, data: &Data) -> AppResult<()> {
    match text(data, "name") {
        Some(name) if !valid_name(name) => Err(ValidationErrors::field(
            "name",
            ctx.t("must be 1-40 lowercase letters, digits and dashes, starting with a letter"),
        )
        .into()),
        _ => Ok(()),
    }
}

async fn deployed_services(txn: &DatabaseTransaction, project: i64) -> AppResult<u64> {
    Ok(service::Entity::find()
        .filter(service::Column::ProjectId.eq(project))
        .filter(service::Column::Status.ne(service::NEW))
        .count(txn)
        .await?)
}
