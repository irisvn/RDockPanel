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
    slug: &str,
) -> AppResult<()> {
    let after = int(data, "service_id");
    if let (Some(before), Some(after)) = (before, after)
        && before != after
    {
        return Err(rustcms::ValidationErrors::field("service_id", ctx.t("cannot change")).into());
    }
    // Created or changed where the caller may (a role's, or their role's in
    // that service's project): Core asks the policy without the record.
    let permission = format!(
        "{slug}.{}",
        if before.is_some() { "update" } else { "create" }
    );
    for id in [before, after].into_iter().flatten() {
        if !parent_ok(ctx, txn, id).await {
            return Err(rustcms::AppError::Forbidden);
        }
        let project = crate::models::service::Entity::find_by_id(id)
            .one(txn)
            .await?
            .map(|s| s.project_id);
        let allowed = match project {
            Some(project) => {
                crate::services::access_service::can(txn, &ctx.principal, project, &permission)
                    .await?
            }
            None => ctx.can(&permission),
        };
        if !allowed {
            return Err(rustcms::AppError::Forbidden);
        }
    }
    Ok(())
}

/// The project a record is of: its project's (`project_id`), or its
/// service's (`service_id`).
pub async fn project_of(ctx: &Ctx, record: Option<&Data>) -> Option<i64> {
    let record = record?;
    if let Some(project) = int(record, "project_id") {
        return Some(project);
    }
    let service = int(record, "service_id")?;
    crate::models::service::Entity::find_by_id(service)
        .one(&ctx.conn())
        .await
        .ok()
        .flatten()
        .map(|s| s.project_id)
}

/// `permission` for the caller on `record`: one of their roles' (every
/// project), else their role's in the record's project (`access_service`).
pub async fn may(ctx: &Ctx, permission: &str, record: Option<&Data>) -> bool {
    if ctx.can(permission) {
        return true;
    }
    match project_of(ctx, record).await {
        Some(project) => {
            crate::services::access_service::can(&ctx.conn(), &ctx.principal, project, permission)
                .await
                .unwrap_or(false)
        }
        None => false,
    }
}

/// `permission` somewhere: a role's, or a project role's.
pub async fn admits(ctx: &Ctx, permission: &str) -> bool {
    crate::services::access_service::admits(&ctx.conn(), &ctx.principal, permission)
        .await
        .unwrap_or(false)
}

/// The user whose records `slug` shows are their projects' only: none when
/// a role of theirs sees every record (`<slug>.view`).
pub fn members_only(ctx: &Ctx, slug: &str) -> Option<i64> {
    // A role's `view` (every record, by key): every record. Else their
    // projects' only — a role's `view_any` alone lists no key or count of
    // another project's.
    if ctx.can(&format!("{slug}.view")) {
        return None;
    }
    // Not a user: nothing (no project has id -1's members).
    Some(ctx.principal.id.parse().unwrap_or(-1))
}

/// A service's records: their permissions (`<slug>.<ability>`, globally or
/// in the service's project), and a privileged service's changed by its
/// administrators only.
pub struct ChildPolicy(pub &'static str);

#[async_trait]
impl Policy for ChildPolicy {
    async fn allows(&self, ctx: &Ctx, ability: &str, record: Option<&Data>) -> bool {
        let permission = format!("{}.{ability}", self.0);
        let allowed = match record {
            Some(_) => may(ctx, &permission, record).await,
            None => admits(ctx, &permission).await,
        };
        if !allowed {
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
        admits(ctx, &format!("{}.{ability}", self.0)).await
    }

    fn view_depends_on_record(&self) -> bool {
        false
    }
}
