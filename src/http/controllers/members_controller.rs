//! A project's members (`/admin/api/x/apps/projects/{id}/members`): users
//! of the panel given a role in it — `viewer`, `developer`, `admin`
//! (`access_service`). Managed by its admins (`projects.members` there) and
//! by those whose roles grant it everywhere. Audited.

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use serde::Deserialize;
use serde_json::Value;

use crate::models::project_member;
use crate::services::access_service::{self, ROLES};

const MEMBERS: &str = "projects.members";

fn audit_context(auth: &Auth, request: RequestMeta) -> AuditContext {
    AuditContext::new(AuditActor::user(auth.principal()), request)
}

async fn shown<C: sea_orm::ConnectionTrait>(db: &C, project: i64) -> AppResult<Vec<Value>> {
    let rows = project_member::Entity::find()
        .filter(project_member::Column::ProjectId.eq(project))
        .order_by_asc(project_member::Column::Id)
        .all(db)
        .await?;
    let users: std::collections::HashMap<i64, rustcms::models::user::Model> =
        rustcms::models::user::Entity::find()
            .filter(rustcms::models::user::Column::Id.is_in(rows.iter().map(|r| r.user_id)))
            .all(db)
            .await?
            .into_iter()
            .map(|u| (u.id, u))
            .collect();
    Ok(rows
        .iter()
        .map(|m| {
            let user = users.get(&m.user_id);
            json!({
                "user_id": m.user_id,
                "name": user.map(|u| u.name.clone()),
                "email": user.map(|u| u.email.clone()),
                "role": m.role,
            })
        })
        .collect())
}

pub async fn index(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    access_service::project(app.db(), auth.principal(), id, MEMBERS).await?;
    Ok(Json(json!({
        "members": shown(app.db(), id).await?,
        "roles": ROLES,
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewMember {
    pub email: String,
    pub role: String,
}

fn parse<T: serde::de::DeserializeOwned>(body: Value) -> AppResult<T> {
    serde_json::from_value(body).map_err(|e| {
        let field = if e.to_string().contains("role") {
            "role"
        } else {
            "email"
        };
        ValidationErrors::field(field, "is required").into()
    })
}

fn check_role(role: &str) -> AppResult<()> {
    if ROLES.contains(&role) {
        Ok(())
    } else {
        Err(ValidationErrors::field("role", "must be viewer, developer or admin").into())
    }
}

/// The admins of `project` other than `user`.
async fn other_admins<C: sea_orm::ConnectionTrait>(
    db: &C,
    project: i64,
    user: i64,
) -> AppResult<u64> {
    Ok(project_member::Entity::find()
        .filter(project_member::Column::ProjectId.eq(project))
        .filter(project_member::Column::Role.eq("admin"))
        .filter(project_member::Column::UserId.ne(user))
        .count(db)
        .await?)
}

/// Gives a user of the panel (by email) a role in the project.
pub async fn add(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    // Allowed first, then the body read (a refusal says nothing of it).
    access_service::project(app.db(), auth.principal(), id, MEMBERS).await?;
    let body: NewMember = parse(body)?;
    let role = body.role.trim();
    check_role(role)?;
    let email = body.email.trim().to_ascii_lowercase();
    let txn = app.db().begin().await?;
    // The project held while its members change.
    crate::models::project::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    // Again under the project's lock: a change of the caller's own role
    // committed meanwhile counts.
    if !access_service::can(&txn, auth.principal(), id, MEMBERS).await? {
        return Err(AppError::Forbidden);
    }
    let user = rustcms::models::user::Entity::find()
        .filter(rustcms::models::user::Column::Email.eq(&email))
        .one(&txn)
        .await?
        .ok_or_else(|| {
            ValidationErrors::field("email", "is not a user of the panel (invite them first)")
        })?;
    if project_member::Entity::find()
        .filter(project_member::Column::ProjectId.eq(id))
        .filter(project_member::Column::UserId.eq(user.id))
        .one(&txn)
        .await?
        .is_some()
    {
        return Err(ValidationErrors::field("email", "is a member already").into());
    }
    let now = chrono::Utc::now();
    project_member::ActiveModel {
        project_id: Set(id),
        user_id: Set(user.id),
        role: Set(role.to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    let entry = AuditEntry::new("projects.member_added", "projects")
        .key(id.to_string())
        .meta(json!({ "user": user.id, "role": role }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    access_service::sync_apps_access(&app, user.id).await?;
    Ok(Json(json!({ "members": shown(app.db(), id).await? })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub role: String,
}

/// The project keeps an admin: its last one is changed or removed only by
/// whoever manages every project's members (else nobody of the project could).
fn last_admin_guard(auth: &Auth, was_admin: bool, others: u64) -> AppResult<()> {
    if was_admin && others == 0 && !auth.principal().can(MEMBERS) {
        return Err(ValidationErrors::field("role", "the project keeps one admin at least").into());
    }
    Ok(())
}

pub async fn update(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path((id, user)): Path<(i64, i64)>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    access_service::project(app.db(), auth.principal(), id, MEMBERS).await?;
    let body: Role = parse(body)?;
    let role = body.role.trim();
    check_role(role)?;
    let txn = app.db().begin().await?;
    crate::models::project::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    // Again under the project's lock: a change of the caller's own role
    // committed meanwhile counts.
    if !access_service::can(&txn, auth.principal(), id, MEMBERS).await? {
        return Err(AppError::Forbidden);
    }
    let row = project_member::Entity::find()
        .filter(project_member::Column::ProjectId.eq(id))
        .filter(project_member::Column::UserId.eq(user))
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if role != "admin" {
        last_admin_guard(
            &auth,
            row.role == "admin",
            other_admins(&txn, id, user).await?,
        )?;
    }
    let before = row.role.clone();
    let mut active: project_member::ActiveModel = row.into();
    active.role = Set(role.to_owned());
    active.updated_at = Set(chrono::Utc::now());
    active.update(&txn).await?;
    let entry = AuditEntry::new("projects.member_changed", "projects")
        .key(id.to_string())
        .meta(json!({ "user": user, "from": before, "to": role }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(json!({ "members": shown(app.db(), id).await? })))
}

pub async fn remove(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path((id, user)): Path<(i64, i64)>,
) -> AppResult<Json<Value>> {
    access_service::project(app.db(), auth.principal(), id, MEMBERS).await?;
    let txn = app.db().begin().await?;
    crate::models::project::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    // Again under the project's lock: a change of the caller's own role
    // committed meanwhile counts.
    if !access_service::can(&txn, auth.principal(), id, MEMBERS).await? {
        return Err(AppError::Forbidden);
    }
    let row = project_member::Entity::find()
        .filter(project_member::Column::ProjectId.eq(id))
        .filter(project_member::Column::UserId.eq(user))
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    last_admin_guard(
        &auth,
        row.role == "admin",
        other_admins(&txn, id, user).await?,
    )?;
    project_member::Entity::delete_by_id(row.id)
        .exec(&txn)
        .await?;
    let entry = AuditEntry::new("projects.member_removed", "projects")
        .key(id.to_string())
        .meta(json!({ "user": user, "role": row.role }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    access_service::sync_apps_access(&app, user).await?;
    Ok(Json(json!({ "members": shown(app.db(), id).await? })))
}

/// Users of the panel whose email or name starts with `q` (at least 2
/// characters; at most 10), to add one: for whoever may see the users
/// (`users.view_any`).
pub async fn users(
    State(app): State<AppState>,
    auth: Auth,
    Query(search): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    use rustcms::models::user;
    // The directory of users is a role's (`users.view_any`); a project's
    // admin adds a member by their exact email.
    if !auth.principal().can("users.view_any") {
        return Err(AppError::Forbidden);
    }
    let q = search
        .get("q")
        .map(|q| q.trim().to_ascii_lowercase())
        .unwrap_or_default();
    // Without the LIKE wildcards first, then at least 2 characters.
    let q = q.replace(['%', '_', '\\'], "");
    if q.chars().count() < 2 {
        return Ok(Json(json!({ "users": [] })));
    }
    let like = format!("{q}%");
    let found = user::Entity::find()
        .filter(
            sea_orm::Condition::any()
                .add(user::Column::Email.like(&like))
                .add(user::Column::Name.like(&like)),
        )
        .order_by_asc(user::Column::Email)
        .limit(10)
        .all(app.db())
        .await?;
    Ok(Json(json!({
        "users": found
            .iter()
            .map(|u| json!({ "id": u.id, "name": u.name, "email": u.email }))
            .collect::<Vec<_>>(),
    })))
}
