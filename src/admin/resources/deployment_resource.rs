//! Deployments: the history of a service's deploys, starts, stops, restarts
//! and destroys, with what each applied. Written by the panel only.

use std::sync::Arc;

use rustcms::admin::*;
use rustcms::{AppError, async_trait};
use sea_orm::EntityTrait;

use crate::jobs::run_deployment::RunDeployment;
use crate::models::{deployment, service};
use crate::services::deployment_service::{self, SUCCEEDED, Trigger};

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

/// Viewing needs the permissions; nobody writes through the panel; rolling
/// back needs `services.deploy`, to a deployment that succeeded running a
/// recorded image (not a stop or a destroy).
fn can_return_to(record: &Data) -> bool {
    let field = |name: &str| record.get(name).and_then(|v| v.as_str());
    field("status") == Some(SUCCEEDED)
        && field("image_digest").is_some_and(|image| !image.is_empty())
        && !matches!(field("trigger"), Some("stop" | "destroy"))
}

/// Whether a deployment's snapshot adds capabilities or sysctls.
fn privileged(record: &Data) -> bool {
    let spec = match record.get("spec") {
        Some(serde_json::Value::String(text)) => {
            serde_json::from_str(text).unwrap_or(serde_json::Value::Null)
        }
        Some(value) => value.clone(),
        None => return true,
    };
    let set = |field: &str| match &spec[field] {
        serde_json::Value::Array(a) => !a.is_empty(),
        serde_json::Value::Object(o) => !o.is_empty(),
        _ => false,
    };
    set("cap_add") || set("sysctls")
}

struct ReadOnly;

#[async_trait]
impl Policy for ReadOnly {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        match ability {
            "view_any" | "view" => ctx.can(&format!("deployments.{ability}")),
            "rollback" => {
                ctx.can("deployments.view")
                    && ctx.can("services.deploy")
                    && record.is_none_or(can_return_to)
                    // Back to added capabilities or sysctls: as setting them.
                    && (record.is_none_or(|r| !privileged(r))
                        || ctx.can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION))
            }
            _ => false,
        }
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        self.allows(ctx, ability, None).await
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
