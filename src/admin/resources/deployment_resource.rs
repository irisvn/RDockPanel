//! Deployments: the history of a service's deploys, starts, stops, restarts
//! and destroys, with what each applied. Written by the panel only.

use std::sync::Arc;

use rustcms::AppError;
use rustcms::admin::*;
use sea_orm::EntityTrait;

use crate::jobs::run_deployment::RunDeployment;
use crate::models::{deployment, service};
use crate::policies::deployment_policy::DeploymentPolicy;
use crate::services::deployment_service::{self, Trigger};

pub struct DeploymentResource;

impl Resource for DeploymentResource {
    type Entity = deployment::Entity;
    const SLUG: &'static str = "deployments";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Deployment")
            .icon("history")
            // A rollback's checks hold the service until it is asked for.
            .lock_parent("service_id", "services")
            .group("Deploy")
            .navigation(false)
            .audit(false)
    }

    fn fields() -> Vec<Field> {
        vec![
            Field::column(deployment::Column::ServiceId)
                .belongs_to("services")
                .guarded()
                .readable(|_| true)
                .filterable(),
            Field::column(deployment::Column::Trigger)
                .guarded()
                .readable(|_| true)
                .filterable(),
            Field::column(deployment::Column::Status)
                .guarded()
                .readable(|_| true)
                .filterable()
                .sortable(),
            Field::column(deployment::Column::ImageDigest)
                .guarded()
                .readable(|_| true),
            Field::column(deployment::Column::Published)
                .guarded()
                .readable(|_| true),
            Field::column(deployment::Column::Spec)
                .guarded()
                .readable(|_| true),
            Field::column(deployment::Column::Log)
                .guarded()
                .readable(|_| true),
            Field::column(deployment::Column::Error)
                .guarded()
                .readable(|_| true),
            Field::column(deployment::Column::UserId)
                .guarded()
                .readable(|_| true),
            Field::column(deployment::Column::StartedAt)
                .guarded()
                .readable(|_| true)
                .sortable(),
            Field::column(deployment::Column::FinishedAt)
                .guarded()
                .readable(|_| true)
                .sortable(),
            Field::column(deployment::Column::CreatedAt)
                .guarded()
                .readable(|_| true)
                .sortable(),
        ]
    }

    fn table() -> Table {
        Table::new()
            .columns(vec![
                TextColumn::make("created_at"),
                TextColumn::make("trigger"),
                TextColumn::make("status"),
                TextColumn::make("service_id"),
                TextColumn::make("finished_at"),
            ])
            .default_sort("-created_at")
    }

    fn policy() -> Arc<dyn Policy> {
        Arc::new(DeploymentPolicy)
    }

    /// A member sees their projects' records only (a role seeing every
    /// record: all of them).
    fn scope(
        query: sea_orm::Select<<Self as Resource>::Entity>,
        ctx: &Ctx,
    ) -> sea_orm::Select<<Self as Resource>::Entity> {
        use sea_orm::{ColumnTrait, QueryFilter};
        match crate::policies::members_only(ctx, "deployments") {
            Some(user) => query.filter(
                crate::models::deployment::Column::ServiceId
                    .in_subquery(crate::services::access_service::member_services(user)),
            ),
            None => query,
        }
    }

    /// "Roll back to this": deploys a succeeded deployment's configuration
    /// and image again (`services.deploy`).
    fn actions() -> Vec<Action<deployment::Entity>> {
        vec![
            Action::<deployment::Entity>::new("rollback", |ctx, txn, deployments, _| {
                Box::pin(async move {
                    let user = ctx.principal.id.parse().ok();
                    for earlier in deployments {
                        let service = service::Entity::find_by_id(earlier.service_id)
                            .one(txn)
                            .await?
                            .ok_or(AppError::NotFound)?;
                        let row = deployment_service::request(
                            txn,
                            &service,
                            Trigger::Rollback,
                            Some(&earlier),
                            user,
                            ctx.keyring()?,
                        )
                        .await?;
                        ctx.dispatch(
                            txn,
                            &RunDeployment {
                                deployment_id: row.id,
                            },
                        )
                        .await?;
                    }
                    Ok(())
                })
            })
            .label("Roll back to this")
            .requires_confirmation("Deploy this configuration and image again?"),
        ]
    }
}
