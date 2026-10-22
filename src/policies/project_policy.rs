//! Projects: the permissions, globally or in the project.

use rustcms::admin::*;
use rustcms::async_trait;
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};

use crate::admin::int;
use crate::models::service;

/// The permissions, and no delete while the project has services.
pub struct ProjectPolicy;

#[async_trait]
impl Policy for ProjectPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        let permission = format!("projects.{ability}");
        // A project's own id is its project.
        let allowed = match record.and_then(|r| int(r, "id")) {
            _ if ctx.can(&permission) => true,
            Some(id) => {
                crate::services::access_service::can(&ctx.conn(), &ctx.principal, id, &permission)
                    .await
                    .unwrap_or(false)
            }
            None => crate::policies::admits(ctx, &permission).await,
        };
        if !allowed {
            return false;
        }
        match (ability, record.and_then(|r| int(r, "id"))) {
            ("delete", Some(id)) => service::Entity::find()
                .filter(service::Column::ProjectId.eq(id))
                // The request's own connection (a delete holds one already).
                .count(&ctx.conn())
                .await
                .is_ok_and(|n| n == 0),
            _ => true,
        }
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        crate::policies::admits(ctx, &format!("projects.{ability}")).await
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
