//! GitHub tokens (`/admin/api/x/apps/git/...`, Settings → Git,
//! `system.manage`): added after GitHub accepts them, sealed, never shown
//! again, each given to the projects that may use it. And for a service's
//! Source tab (`services.update`, `/apps/services/{id}/github/...`): the
//! tokens its project was given, the repositories and branches one reaches.

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::crypto::Encrypted;
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use serde::Deserialize;
use serde_json::Value;

use crate::http::controllers::apps_controller::forbid_privileged;
use crate::models::{git_provider, project, service};
use crate::services::github_service::GitHub;

fn github(app: &AppState) -> GitHub {
    app.service::<GitHub>()
        .map(|g| (*g).clone())
        .unwrap_or_default()
}

fn audit_context(auth: &Auth, request: RequestMeta) -> AuditContext {
    AuditContext::new(AuditActor::user(auth.principal()), request)
}

fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.chars().count() <= 64
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
}

fn valid_token(token: &str) -> bool {
    let token = token.trim();
    !token.is_empty() && token.len() <= 255 && token.chars().all(|c| c.is_ascii_graphic())
}

fn open(app: &AppState, row: &git_provider::Model) -> AppResult<String> {
    row.token
        .open(app.keyring(), &git_provider::token_context())
        .map_err(|_| {
            ValidationErrors::field(
                "token",
                "cannot be read (sealed with another APP_KEY): set it again",
            )
            .into()
        })
}

/// What a token is shown as (never its value).
fn shown(row: &git_provider::Model, used_by: u64) -> Value {
    json!({
        "id": row.id,
        "name": row.name,
        "kind": row.kind,
        "account": row.account,
        "projects": row.projects(),
        "used_by": used_by,
        "updated_at": row.updated_at,
    })
}

async fn used_by<C: sea_orm::ConnectionTrait>(db: &C, id: i64) -> AppResult<u64> {
    Ok(service::Entity::find()
        .filter(service::Column::GitProviderId.eq(id))
        .count(db)
        .await?)
}

pub async fn index(State(app): State<AppState>) -> AppResult<Json<Value>> {
    let db = app.db();
    let mut providers = vec![];
    for row in git_provider::Entity::find()
        .order_by_asc(git_provider::Column::Name)
        .all(db)
        .await?
    {
        let n = used_by(db, row.id).await?;
        providers.push(shown(&row, n));
    }
    let projects: Vec<Value> = project::Entity::find()
        .order_by_asc(project::Column::Name)
        .all(db)
        .await?
        .into_iter()
        .map(|p| json!({ "id": p.id, "name": p.name }))
        .collect();
    Ok(Json(
        json!({ "providers": providers, "projects": projects }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub name: Option<String>,
    pub token: Option<String>,
    pub projects: Option<Vec<i64>>,
}

/// The projects among `ids` that exist, sorted.
async fn projects_of<C: sea_orm::ConnectionTrait>(db: &C, ids: &[i64]) -> AppResult<String> {
    let mut found: Vec<i64> = project::Entity::find()
        .filter(project::Column::Id.is_in(ids.iter().copied()))
        .all(db)
        .await?
        .into_iter()
        .map(|p| p.id)
        .collect();
    found.sort_unstable();
    Ok(serde_json::to_string(&found).expect("ids serialize"))
}

pub async fn create(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Json(body): Json<Provider>,
) -> AppResult<Json<Value>> {
    let name = body.name.unwrap_or_default().trim().to_owned();
    let token = body.token.unwrap_or_default().trim().to_owned();
    let mut errors = ValidationErrors::new();
    if !valid_name(&name) {
        errors.add("name", "must be 1 to 64 letters, digits, spaces, - _ .");
    }
    if !valid_token(&token) {
        errors.add("token", "is required (a GitHub token, no spaces)");
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }
    // GitHub must accept it first: who it signs in as.
    let checked = github(&app).check(&token).await?;
    let db = app.db();
    let txn = db.begin().await?;
    if git_provider::Entity::find()
        .filter(git_provider::Column::Name.eq(&name))
        .one(&txn)
        .await?
        .is_some()
    {
        return Err(ValidationErrors::field("name", "is taken").into());
    }
    let now = chrono::Utc::now();
    let row = git_provider::ActiveModel {
        name: Set(name),
        kind: Set("github".to_owned()),
        token: Set(
            Encrypted::seal(app.keyring(), &git_provider::token_context(), &token)
                .map_err(AppError::internal)?,
        ),
        account: Set(checked["account"].as_str().map(str::to_owned)),
        project_ids: Set(projects_of(&txn, &body.projects.unwrap_or_default()).await?),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&txn)
    .await?;
    let entry = AuditEntry::new("git_providers.create", "git_providers")
        .key(row.id.to_string())
        .meta(json!({ "name": row.name, "account": row.account, "projects": row.projects() }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(
        json!({ "provider": shown(&row, 0), "scopes": checked["scopes"] }),
    ))
}

/// Renames it, replaces its token (checked by GitHub first), or changes
/// the projects given it (a service of a project taken from it fails its
/// next deployment, said so).
pub async fn update(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<Provider>,
) -> AppResult<Json<Value>> {
    let token = body
        .token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    if token.is_some_and(|t| !valid_token(t)) {
        return Err(ValidationErrors::field("token", "is not a GitHub token (no spaces)").into());
    }
    if body.name.as_deref().is_some_and(|n| !valid_name(n)) {
        return Err(ValidationErrors::field(
            "name",
            "must be 1 to 64 letters, digits, spaces, - _ .",
        )
        .into());
    }
    let checked = match token {
        Some(token) => Some(github(&app).check(token).await?),
        None => None,
    };
    let db = app.db();
    let txn = db.begin().await?;
    let row = git_provider::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    let mut active: git_provider::ActiveModel = row.clone().into();
    if let Some(name) = body.name.as_deref().map(str::trim)
        && name != row.name
    {
        if git_provider::Entity::find()
            .filter(git_provider::Column::Name.eq(name))
            .one(&txn)
            .await?
            .is_some()
        {
            return Err(ValidationErrors::field("name", "is taken").into());
        }
        active.name = Set(name.to_owned());
    }
    if let (Some(token), Some(checked)) = (token, &checked) {
        active.token = Set(Encrypted::seal(
            app.keyring(),
            &git_provider::token_context(),
            &token.to_owned(),
        )
        .map_err(AppError::internal)?);
        active.account = Set(checked["account"].as_str().map(str::to_owned));
    }
    if let Some(projects) = &body.projects {
        active.project_ids = Set(projects_of(&txn, projects).await?);
    }
    active.updated_at = Set(chrono::Utc::now());
    let row = active.update(&txn).await?;
    let entry = AuditEntry::new("git_providers.update", "git_providers")
        .key(row.id.to_string())
        .meta(json!({
            "name": row.name,
            "token_replaced": token.is_some(),
            "projects": row.projects(),
        }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    let n = used_by(db, row.id).await?;
    Ok(Json(json!({ "provider": shown(&row, n) })))
}

/// Removes it, unless a service clones with it.
pub async fn delete(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let db = app.db();
    let txn = db.begin().await?;
    let row = git_provider::Entity::find_by_id(id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    let n = used_by(&txn, id).await?;
    if n > 0 {
        return Err(ValidationErrors::field(
            "provider",
            format!("{n} service(s) clone with it: choose another source there first"),
        )
        .into());
    }
    git_provider::Entity::delete_by_id(id).exec(&txn).await?;
    let entry = AuditEntry::new("git_providers.delete", "git_providers")
        .key(id.to_string())
        .meta(json!({ "name": row.name }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(json!({ "deleted": true })))
}

/// Asks GitHub who the stored token signs in as (kept as its account).
pub async fn test(State(app): State<AppState>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    let row = git_provider::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    let checked = github(&app).check(&open(&app, &row)?).await?;
    let account = checked["account"].as_str().map(str::to_owned);
    if account != row.account {
        let mut active: git_provider::ActiveModel = row.into();
        active.account = Set(account);
        active.update(app.db()).await?;
    }
    Ok(Json(checked))
}

/// The service, for a user who may change it (its Source).
async fn editable(app: &AppState, auth: &Auth, id: i64) -> AppResult<service::Model> {
    // A role's, or the caller's role in its project.
    let row =
        crate::services::access_service::service(app.db(), auth.principal(), id, "services.update")
            .await?;
    forbid_privileged(auth, &row)?;
    Ok(row)
}

/// The tokens the service's project was given (names, accounts).
pub async fn for_service(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = editable(&app, &auth, id).await?;
    let providers: Vec<Value> = git_provider::Entity::find()
        .order_by_asc(git_provider::Column::Name)
        .all(app.db())
        .await?
        .into_iter()
        .filter(|p| p.allows(row.project_id))
        .map(|p| json!({ "id": p.id, "name": p.name, "account": p.account }))
        .collect();
    Ok(Json(json!({ "providers": providers })))
}

/// A token the service's project was given, opened.
async fn token_for(app: &AppState, auth: &Auth, id: i64, provider: i64) -> AppResult<String> {
    let row = editable(app, auth, id).await?;
    let found = git_provider::Entity::find_by_id(provider)
        .one(app.db())
        .await?
        .filter(|p| p.allows(row.project_id))
        .ok_or(AppError::NotFound)?;
    open(app, &found)
}

pub async fn repositories(
    State(app): State<AppState>,
    auth: Auth,
    Path((id, provider)): Path<(i64, i64)>,
) -> AppResult<Json<Value>> {
    let token = token_for(&app, &auth, id, provider).await?;
    Ok(Json(
        json!({ "repositories": github(&app).repositories(&token).await? }),
    ))
}

pub async fn branches(
    State(app): State<AppState>,
    auth: Auth,
    Path((id, provider)): Path<(i64, i64)>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> AppResult<Json<Value>> {
    // Allowed first, then the query read.
    let token = token_for(&app, &auth, id, provider).await?;
    let (owner, repo) = query
        .get("repo")
        .map(String::as_str)
        .unwrap_or_default()
        .split_once('/')
        .ok_or_else(|| ValidationErrors::field("repo", "must be owner/name"))?;
    Ok(Json(
        json!({ "branches": github(&app).branches(&token, owner, repo).await? }),
    ))
}
