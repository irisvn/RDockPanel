//! Who may do what in a project: a permission of the user's roles (global:
//! every project), or one their role in the project grants (`project_members`:
//! `viewer`, `developer`, `admin`). A member sees and acts on their projects
//! only. `system.manage` is never a project's to grant (a privileged
//! service stays its administrators', whatever the project role).

use rustcms::{AppError, AppResult, Principal};
use sea_orm::sea_query::{Query, SelectStatement};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

use crate::models::{project_member, service};

/// Opening the apps pages, nothing more: a member's (their projects say
/// what they see there).
pub const APPS_ACCESS: &str = "apps.access";

/// Gives `user` [`APPS_ACCESS`] while they are a member of some project;
/// takes it back once they are none's (a role's grant stays).
pub async fn sync_apps_access(app: &rustcms::AppState, user: i64) -> AppResult<()> {
    use rustcms::auth::{ADMIN_GUARD, Actor, AuthState, RbacService};
    let Some(auth) = app.service::<AuthState>() else {
        return Ok(());
    };
    let rbac = RbacService::new(app.db(), &auth.resolver);
    if memberships(app.db(), user).await?.is_empty() {
        rbac.revoke_permission(Actor::System, user, ADMIN_GUARD, APPS_ACCESS)
            .await
    } else {
        rbac.grant_permission(Actor::System, user, ADMIN_GUARD, APPS_ACCESS)
            .await
    }
}

/// The roles, least to most.
pub const ROLES: [&str; 3] = ["viewer", "developer", "admin"];

/// What a viewer may: see the project, its services and their records.
const VIEWER: &[&str] = &[
    "projects.view",
    "projects.view_any",
    "services.view",
    "services.view_any",
    "services.logs",
    "deployments.view",
    "deployments.view_any",
    "service_domains.view",
    "service_domains.view_any",
    "service_ports.view",
    "service_ports.view_any",
    "service_mounts.view",
    "service_mounts.view_any",
    "service_redirects.view",
    "service_redirects.view_any",
    "service_scripts.view",
    "service_scripts.view_any",
    "database_backups.view",
    "database_backups.view_any",
];

/// A developer's, besides a viewer's: change, deploy and run its services
/// (their secrets, their terminal), not remove them.
const DEVELOPER: &[&str] = &[
    "services.create",
    "services.update",
    "services.deploy",
    "services.secrets",
    "services.exec",
    "deployments.rollback",
    "service_domains.create",
    "service_domains.update",
    "service_domains.delete",
    "service_ports.create",
    "service_ports.update",
    "service_ports.delete",
    "service_mounts.create",
    "service_mounts.update",
    "service_mounts.delete",
    "service_redirects.create",
    "service_redirects.update",
    "service_redirects.delete",
    "service_scripts.create",
    "service_scripts.update",
    "service_scripts.delete",
    "database_backups.create",
    "database_backups.update",
    "database_backups.delete",
    "backups.run",
];

/// An admin's, besides a developer's: remove services, restore databases,
/// rename the project, choose its members.
const ADMIN: &[&str] = &[
    "services.delete",
    "services.destroy",
    "backups.restore",
    "projects.update",
    "projects.members",
];

/// Every permission a project role may grant (what a project's page asks).
pub fn project_permissions() -> impl Iterator<Item = &'static str> {
    VIEWER.iter().chain(DEVELOPER).chain(ADMIN).copied()
}

/// Whether `role` grants `permission` (never `system.*`).
pub fn role_grants(role: &str, permission: &str) -> bool {
    if permission.starts_with("system.") {
        return false;
    }
    let lists: &[&[&str]] = match role {
        "viewer" => &[VIEWER],
        "developer" => &[VIEWER, DEVELOPER],
        "admin" => &[VIEWER, DEVELOPER, ADMIN],
        _ => &[],
    };
    lists.iter().any(|list| list.contains(&permission))
}

/// The user a principal is (none: not a user).
pub fn user_of(principal: &Principal) -> Option<i64> {
    principal.id.parse().ok()
}

/// `user`'s memberships: (project, role).
pub async fn memberships<C: ConnectionTrait>(db: &C, user: i64) -> AppResult<Vec<(i64, String)>> {
    Ok(project_member::Entity::find()
        .filter(project_member::Column::UserId.eq(user))
        .filter(project_member::Column::ProjectId.in_subquery(existing_projects()))
        .all(db)
        .await?
        .into_iter()
        .map(|m| (m.project_id, m.role))
        .collect())
}

/// Whether `principal` may do `permission` in `project`.
pub async fn can<C: ConnectionTrait>(
    db: &C,
    principal: &Principal,
    project: i64,
    permission: &str,
) -> AppResult<bool> {
    if principal.can(permission) {
        return Ok(true);
    }
    let Some(user) = user_of(principal) else {
        return Ok(false);
    };
    Ok(project_member::Entity::find()
        .filter(project_member::Column::UserId.eq(user))
        .filter(project_member::Column::ProjectId.eq(project))
        .filter(project_member::Column::ProjectId.in_subquery(existing_projects()))
        .one(db)
        .await?
        .is_some_and(|m| role_grants(&m.role, permission)))
}

/// Whether `principal` may do `permission` somewhere (globally, or in a
/// project of theirs).
pub async fn admits<C: ConnectionTrait>(
    db: &C,
    principal: &Principal,
    permission: &str,
) -> AppResult<bool> {
    if principal.can(permission) {
        return Ok(true);
    }
    let Some(user) = user_of(principal) else {
        return Ok(false);
    };
    Ok(memberships(db, user)
        .await?
        .iter()
        .any(|(_, role)| role_grants(role, permission)))
}

/// The projects where `principal` may do `permission`: `None` for all
/// (a global permission).
pub async fn projects_where<C: ConnectionTrait>(
    db: &C,
    principal: &Principal,
    permission: &str,
) -> AppResult<Option<Vec<i64>>> {
    if principal.can(permission) {
        return Ok(None);
    }
    let Some(user) = user_of(principal) else {
        return Ok(Some(vec![]));
    };
    Ok(Some(
        memberships(db, user)
            .await?
            .into_iter()
            .filter(|(_, role)| role_grants(role, permission))
            .map(|(project, _)| project)
            .collect(),
    ))
}

/// `SELECT project_id FROM project_members WHERE user_id = user` (for the
/// resources' scopes, which are not async).
pub fn member_projects(user: i64) -> SelectStatement {
    Query::select()
        .column(project_member::Column::ProjectId)
        .from(project_member::Entity)
        .and_where(project_member::Column::UserId.eq(user))
        .to_owned()
}

/// `SELECT id FROM projects`: a membership of a project deleted since
/// counts for nothing (until [`forget_deleted_projects`] removes it).
fn existing_projects() -> SelectStatement {
    Query::select()
        .column(crate::models::project::Column::Id)
        .from(crate::models::project::Entity)
        .to_owned()
}

/// The memberships of projects deleted since: removed, and their users'
/// `apps.access` given again as their memberships now say (at boot).
pub async fn forget_deleted_projects(app: &rustcms::AppState) -> AppResult<usize> {
    let orphans = project_member::Entity::find()
        .filter(project_member::Column::ProjectId.not_in_subquery(existing_projects()))
        .all(app.db())
        .await?;
    for row in &orphans {
        project_member::Entity::delete_by_id(row.id)
            .exec(app.db())
            .await?;
    }
    let mut users: Vec<i64> = orphans.iter().map(|r| r.user_id).collect();
    users.sort_unstable();
    users.dedup();
    for user in users {
        sync_apps_access(app, user).await?;
    }
    Ok(orphans.len())
}

/// `SELECT id FROM services WHERE project_id IN (member_projects)`.
pub fn member_services(user: i64) -> SelectStatement {
    Query::select()
        .column(service::Column::Id)
        .from(service::Entity)
        .and_where(service::Column::ProjectId.in_subquery(member_projects(user)))
        .to_owned()
}

/// What a caller may in one project, read once: [`Checker::can`] asks no
/// more of the database.
pub struct Checker<'a> {
    principal: &'a Principal,
    role: Option<String>,
}

impl Checker<'_> {
    /// A role's permission, or the caller's role in the project's.
    pub fn can(&self, permission: &str) -> bool {
        self.principal.can(permission)
            || self
                .role
                .as_deref()
                .is_some_and(|r| role_grants(r, permission))
    }

    /// The caller's role in the project (none: not a member).
    pub fn role(&self) -> Option<&str> {
        self.role.as_deref()
    }
}

/// [`Checker`] of `principal` in `project`.
pub async fn checker<'a, C: ConnectionTrait>(
    db: &C,
    principal: &'a Principal,
    project: i64,
) -> AppResult<Checker<'a>> {
    let role = match user_of(principal) {
        Some(user) => project_member::Entity::find()
            .filter(project_member::Column::UserId.eq(user))
            .filter(project_member::Column::ProjectId.eq(project))
            .filter(project_member::Column::ProjectId.in_subquery(existing_projects()))
            .one(db)
            .await?
            .map(|m| m.role),
        None => None,
    };
    Ok(Checker { principal, role })
}

/// The service `id` for `permission`: 403 when the caller may not do it
/// anywhere (before any lookup: ids are not probed), 404 when it does not
/// exist, 403 when not in this service's project.
pub async fn service<C: ConnectionTrait>(
    db: &C,
    principal: &Principal,
    id: i64,
    permission: &str,
) -> AppResult<service::Model> {
    if !admits(db, principal, permission).await? {
        return Err(AppError::Forbidden);
    }
    let row = service::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    if !can(db, principal, row.project_id, permission).await? {
        return Err(AppError::Forbidden);
    }
    Ok(row)
}

/// The service `id` for any of `permissions` (the first that allows).
pub async fn service_any<C: ConnectionTrait>(
    db: &C,
    principal: &Principal,
    id: i64,
    permissions: &[&str],
) -> AppResult<service::Model> {
    let mut last = AppError::Forbidden;
    for permission in permissions {
        match service(db, principal, id, permission).await {
            Ok(row) => return Ok(row),
            Err(err @ AppError::NotFound) => return Err(err),
            Err(err) => last = err,
        }
    }
    Err(last)
}

/// A live stream's (or terminal's) check of the service `id` for
/// `permission`, again at every recheck: a member removed, or a role taken
/// away, ends it.
pub fn service_target(
    db: sea_orm::DatabaseConnection,
    id: i64,
    permission: &'static str,
) -> impl Fn(Principal) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<()>> + Send>>
+ Send
+ Sync
+ 'static {
    move |principal| {
        let db = db.clone();
        Box::pin(async move { service(&db, &principal, id, permission).await.map(drop) })
    }
}

/// [`service_target`] for any of `permissions`.
pub fn service_target_any(
    db: sea_orm::DatabaseConnection,
    id: i64,
    permissions: &'static [&'static str],
) -> impl Fn(Principal) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<()>> + Send>>
+ Send
+ Sync
+ 'static {
    move |principal| {
        let db = db.clone();
        Box::pin(async move {
            service_any(&db, &principal, id, permissions)
                .await
                .map(drop)
        })
    }
}

/// The project `id` for `permission` (as [`service`]).
pub async fn project<C: ConnectionTrait>(
    db: &C,
    principal: &Principal,
    id: i64,
    permission: &str,
) -> AppResult<crate::models::project::Model> {
    if !admits(db, principal, permission).await? {
        return Err(AppError::Forbidden);
    }
    let row = crate::models::project::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    if !can(db, principal, row.id, permission).await? {
        return Err(AppError::Forbidden);
    }
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles() {
        assert!(role_grants("viewer", "services.view"));
        assert!(!role_grants("viewer", "services.update"));
        assert!(role_grants("developer", "services.deploy"));
        assert!(role_grants("developer", "services.secrets"));
        assert!(!role_grants("developer", "services.destroy"));
        assert!(role_grants("admin", "services.destroy"));
        assert!(role_grants("admin", "projects.members"));
        for role in ROLES {
            assert!(!role_grants(role, "system.manage"));
            assert!(!role_grants(role, "projects.create"));
            assert!(!role_grants(role, "docker.view"));
        }
        assert!(!role_grants("owner", "services.view"));
    }
}
