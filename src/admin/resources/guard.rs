//! A privileged service — added capabilities, sysctls or groups, or a
//! Compose service — is the panel's administrators' only (`system.manage`):
//! the service and every record of it (domains, ports, mounts, redirects,
//! scripts, backups). [`ChildPolicy`] and [`write`] check the records.

use rustcms::admin::*;
use rustcms::{AppResult, async_trait};
use sea_orm::{ConnectionTrait, DatabaseTransaction, EntityTrait};

use crate::admin::int;
use crate::admin::resources::service_resource::privileged;

fn system(ctx: &Ctx) -> bool {
    ctx.can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION)
}

/// Whether the service `id` may be changed by the caller (unknown: yes, its
/// own rules answer).
pub async fn parent_ok<C: ConnectionTrait>(ctx: &Ctx, conn: &C, id: i64) -> bool {
    if system(ctx) {
        return true;
    }
    match crate::models::service::Entity::find_by_id(id)
        .one(conn)
        .await
    {
        Ok(found) => !found.is_some_and(|s| privileged(&s)),
        Err(_) => false,
    }
}

/// A record written: its service (and, updated, the one it had) may be
/// changed by the caller, and it never moves to another service.
pub async fn write(
    ctx: &Ctx,
    txn: &DatabaseTransaction,
    data: &Data,
    before: Option<i64>,
) -> AppResult<()> {
    let after = int(data, "service_id");
    if let (Some(before), Some(after)) = (before, after)
        && before != after
    {
        return Err(rustcms::ValidationErrors::field("service_id", ctx.t("cannot change")).into());
    }
    for id in [before, after].into_iter().flatten() {
        if !parent_ok(ctx, txn, id).await {
            return Err(rustcms::AppError::Forbidden);
        }
    }
    Ok(())
}

/// A service's records: their permissions (`<slug>.<ability>`), and a
/// privileged service's changed by its administrators only.
pub struct ChildPolicy(pub &'static str);

#[async_trait]
impl Policy for ChildPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        if !ctx.can(&format!("{}.{ability}", self.0)) {
            return false;
        }
        if matches!(ability, "view" | "view_any") {
            return true;
        }
        match record.and_then(|r| int(r, "service_id")) {
            Some(id) => parent_ok(ctx, &ctx.conn(), id).await,
            None => true,
        }
    }

    async fn admits(&self, ctx: &Ctx, ability: &str) -> bool {
        ctx.can(&format!("{}.{ability}", self.0))
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
