//! Deployments: the history of a service's deploys, starts, stops, restarts
//! and destroys, with what each applied. Written by the panel only.

use std::sync::Arc;

use rustcms::admin::*;
use rustcms::async_trait;

use crate::models::deployment;

pub struct DeploymentResource;

impl Resource for DeploymentResource {
    type Entity = deployment::Entity;
    const SLUG: &'static str = "deployments";

    fn meta() -> ResourceMeta {
        ResourceMeta::new("Deployment")
            .icon("history")
            .group("Deploy")
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
        Arc::new(ReadOnly)
    }
}

/// Viewing needs the permissions; nobody writes through the panel.
struct ReadOnly;

#[async_trait]
impl Policy for ReadOnly {
    async fn allows(&self, ctx: &Ctx, ability: &str, _record: Option<&Data>) -> bool {
        matches!(ability, "view_any" | "view") && ctx.can(&format!("deployments.{ability}"))
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        self.allows(ctx, ability, None).await
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
