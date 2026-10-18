//! The apps interface's reads (`routes/admin.rs`, under `/admin/api/x/apps`):
//! the dashboard's projects and services, a service's page, and its
//! environment as `.env` text. Writes go through the resources' API (their
//! policies, validation and audit), except the environment's text.

use std::collections::BTreeMap;

use rustcms::audit::{self, AuditActor, AuditContext, AuditEntry, RequestMeta};
use rustcms::crypto::Encrypted;
use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json, ValidationErrors};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, TransactionTrait};
use serde::Deserialize;
use serde_json::Value;

use crate::models::{deployment, project, service, service_domain, service_mount, service_port};
use crate::services::creation_service::{self, DatabaseOptions, NewService};
use crate::services::database_service::Engine;
use crate::services::deployment_service::{
    has_pending_changes, placeholder_vars, service_env, snapshot_without_env, summary,
};
use crate::services::dotenv;

/// Reading environment values (an audited read): the Environment tab.
pub const SECRETS_PERMISSION: &str = "services.secrets";

/// The most variables a service takes.
const MAX_ENV: usize = 100;
/// The longest value.
const MAX_ENV_VALUE: usize = 16_384;

fn primary_host(domains: &[service_domain::Model]) -> Option<String> {
    domains.first().map(|d| d.host.clone())
}

/// Every project with its services, for the dashboard: each kind of
/// record only with its resource's permission (services, domains).
pub async fn overview(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    let db = app.db();
    let principal = auth.principal();
    let see_services = principal.can("services.view_any");
    let see_domains = principal.can("service_domains.view_any");
    let projects = project::Entity::find()
        .order_by_asc(project::Column::Name)
        .all(db)
        .await?;
    let services = if see_services {
        service::Entity::find()
            .order_by_asc(service::Column::Name)
            .all(db)
            .await?
    } else {
        vec![]
    };
    let domains = if see_domains {
        service_domain::Entity::find()
            .order_by_asc(service_domain::Column::Id)
            .all(db)
            .await?
    } else {
        vec![]
    };
    let projects: Vec<Value> = projects
        .iter()
        .map(|p| {
            let services: Vec<Value> = services
                .iter()
                .filter(|s| s.project_id == p.id)
                .map(|s| {
                    let own: Vec<service_domain::Model> = domains
                        .iter()
                        .filter(|d| d.service_id == s.id)
                        .cloned()
                        .collect();
                    json!({
                        "id": s.id,
                        "name": s.name,
                        "kind": s.kind,
                        "image": s.image,
                        "status": s.status,
                        "desired_state": s.desired_state,
                        "busy": s.current_deployment_id.is_some(),
                        "domain": primary_host(&own),
                    })
                })
                .collect();
            json!({
                "id": p.id,
                "name": p.name,
                "description": p.description,
                "services": services,
            })
        })
        .collect();
    Ok(Json(
        json!({ "projects": projects, "services_visible": see_services }),
    ))
}

async fn service_of(app: &AppState, id: i64) -> AppResult<service::Model> {
    service::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)
}

/// A service's page: its configuration (no secret value), domains, ports,
/// mounts, state and recent deployments.
pub async fn service(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let db = app.db();
    let row = service_of(&app, id).await?;
    let principal = auth.principal();
    let project = project::Entity::find_by_id(row.project_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    let snapshot = snapshot_without_env(db, &row).await?;
    // Related records, each with its own resource's permission.
    let domains = if principal.can("service_domains.view_any") {
        service_domain::Entity::find()
            .filter(service_domain::Column::ServiceId.eq(id))
            .order_by_asc(service_domain::Column::Id)
            .all(db)
            .await?
    } else {
        vec![]
    };
    let redirects = if principal.can("service_redirects.view_any") {
        crate::models::service_redirect::Entity::find()
            .filter(crate::models::service_redirect::Column::ServiceId.eq(id))
            .order_by_asc(crate::models::service_redirect::Column::Id)
            .all(db)
            .await?
    } else {
        vec![]
    };
    // Users of its basic authentication: their names (never a hash).
    let auth_users: Vec<Value> = crate::models::service_basic_auth::Entity::find()
        .filter(crate::models::service_basic_auth::Column::ServiceId.eq(id))
        .order_by_asc(crate::models::service_basic_auth::Column::Username)
        .all(db)
        .await?
        .into_iter()
        .map(|u| json!({ "id": u.id, "username": u.username }))
        .collect();
    let ports = if principal.can("service_ports.view_any") {
        service_port::Entity::find()
            .filter(service_port::Column::ServiceId.eq(id))
            .order_by_asc(service_port::Column::Id)
            .all(db)
            .await?
    } else {
        vec![]
    };
    let mounts = if principal.can("service_mounts.view_any") {
        service_mount::Entity::find()
            .filter(service_mount::Column::ServiceId.eq(id))
            .order_by_asc(service_mount::Column::Id)
            .all(db)
            .await?
    } else {
        vec![]
    };
    // Names only: values need `services.secrets` (`/env`).
    let (env_names, env_readable) = match service_env(&row, app.keyring()) {
        Ok(env) => (env.into_keys().collect::<Vec<_>>(), true),
        Err(_) => (vec![], false),
    };
    let can_deployments = principal.can("deployments.view");
    let deployments: Vec<Value> = if can_deployments {
        deployment::Entity::find()
            .filter(deployment::Column::ServiceId.eq(id))
            .order_by_desc(deployment::Column::Id)
            .limit(20)
            .all(db)
            .await?
            .iter()
            .map(|d| {
                let mut s = summary(d);
                s["log"] = Value::Null;
                s["created_at"] = json!(d.created_at);
                s["finished_at"] = json!(d.finished_at);
                s
            })
            .collect()
    } else {
        vec![]
    };
    let mut latest = if can_deployments {
        crate::services::deployment_service::latest(db, id)
            .await?
            .map(|d| summary(&d))
    } else {
        None
    };
    if let Some(latest) = latest.as_mut() {
        latest["log"] = Value::Null;
    }
    Ok(Json(json!({
        "service": {
            "id": row.id,
            "name": row.name,
            "kind": row.kind,
            "image": row.image,
            "command": row.command,
            "replicas": row.replicas,
            "cpu_limit_millis": row.cpu_limit_millis,
            "mem_limit_mb": row.mem_limit_mb,
            "cpu_reservation_millis": row.cpu_reservation_millis,
            "mem_reservation_mb": row.mem_reservation_mb,
            "zero_downtime": row.zero_downtime,
            "healthcheck_cmd": row.healthcheck_cmd,
            "healthcheck_interval_s": row.healthcheck_interval_s,
            "healthcheck_retries": row.healthcheck_retries,
            "registry_username": row.registry_username,
            "source": row.source,
            "git_url": row.git_url,
            "git_branch": row.git_branch,
            "git_path": row.git_path,
            "builder": row.builder,
            "dockerfile_path": row.dockerfile_path,
            "dockerfile": row.dockerfile,
            "compose": row.compose,
            "notes": row.notes,
            "webhook_set": row.webhook_secret.is_some(),
            "compose_file": row.compose_file,
            "deploy_key_public": row.deploy_key_public,
            "trigger_set": row.trigger_hash.is_some(),
            "registry_password_set": row.registry_password.is_some(),
            "status": row.status,
            "desired_state": row.desired_state,
            "deployed_at": row.deployed_at,
            "busy": row.current_deployment_id.is_some(),
        },
        // Its project's name is the service's address (`<project>_<service>`).
        "project": { "id": project.id, "name": project.name },
        "swarm_name": snapshot.service_name(),
        "internal_host": snapshot.service_name(),
        // `<project>-<service>.<[panel] service_domain>`, when set.
        "auto_domain": placeholder_vars(&snapshot).auto_domain(),
        "pending": has_pending_changes(db, &row, app.keyring()).await?,
        "zero_downtime_possible": snapshot.zero_downtime_possible(),
        "env_names": env_names,
        "database": database_facts(&app, &row, &snapshot.service_name()),
        "env_readable": env_readable,
        "domains": domains,
        "redirects": redirects,
        "basic_auth": auth_users,
        "ports": ports,
        "mounts": mounts,
        "latest": latest,
        "deployments": deployments,
        "can": {
            "update": principal.can("services.update"),
            "delete": principal.can("services.delete"),
            "deploy": principal.can("services.deploy"),
            "destroy": principal.can("services.destroy"),
            "secrets": principal.can(SECRETS_PERMISSION),
            "deployments": can_deployments,
            "domains": {
                "view": principal.can("service_domains.view_any"),
                "create": principal.can("service_domains.create"),
                "delete": principal.can("service_domains.delete"),
            },
            "ports": {
                "view": principal.can("service_ports.view_any"),
                "create": principal.can("service_ports.create"),
                "delete": principal.can("service_ports.delete"),
            },
            "mounts": {
                "view": principal.can("service_mounts.view_any"),
                "create": principal.can("service_mounts.create"),
                "delete": principal.can("service_mounts.delete"),
            },
        },
    })))
}

fn audit_context(auth: &Auth, request: RequestMeta) -> AuditContext {
    AuditContext::new(AuditActor::user(auth.principal()), request)
}

/// The environment as `.env` text (values in clear: an audited read).
pub async fn env(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = service_of(&app, id).await?;
    let env = service_env(&row, app.keyring()).map_err(|_| {
        AppError::BadRequest(
            "the environment cannot be read (sealed with another APP_KEY): set it again".into(),
        )
    })?;
    let entry = AuditEntry::new("services.env.read", "services")
        .key(id.to_string())
        .meta(json!({ "names": env.keys().collect::<Vec<_>>() }));
    // No audit, no values.
    let txn = app.db().begin().await?;
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(json!({
        "text": dotenv::format(&env),
        "revision": env_revision(&app, &row),
    })))
}

/// What the environment is now (changes with every write; tells nothing of
/// it): a save must name the revision it was read at.
fn env_revision(app: &AppState, row: &service::Model) -> String {
    row.env.as_ref().map_or_else(
        || "none".to_owned(),
        |sealed| app.keyring().revision(sealed.envelope()),
    )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvText {
    pub text: String,
    /// From the read (`GET …/env`); `None` replaces whatever is there (for
    /// users who may not read the values).
    #[serde(default)]
    pub revision: Option<String>,
}

/// Replaces the environment with `.env` text (sealed; audited by names).
/// A change to deploy, like any other.
pub async fn put_env(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<EnvText>,
) -> AppResult<Json<Value>> {
    let row = service_of(&app, id).await?;
    // A Compose file reads its environment (`${VAR}`, even in its volumes):
    // the panel's administrators only, as the file itself.
    if row.kind == "compose"
        && !auth
            .principal()
            .can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION)
    {
        return Err(AppError::Forbidden);
    }
    let env: BTreeMap<String, String> = dotenv::parse(&body.text).map_err(|e| {
        AppError::from(ValidationErrors::field(
            "text",
            format!("line {}: {}", e.line, e.message),
        ))
    })?;
    if env.len() > MAX_ENV {
        return Err(ValidationErrors::field("text", format!("at most {MAX_ENV} variables")).into());
    }
    if let Some((name, _)) = env.iter().find(|(_, v)| v.len() > MAX_ENV_VALUE) {
        return Err(ValidationErrors::field(
            "text",
            format!("{name}: a value is at most {MAX_ENV_VALUE} characters"),
        )
        .into());
    }
    let sealed = (!env.is_empty())
        .then(|| Encrypted::seal(app.keyring(), &service::env_context(), &env))
        .transpose()?;
    let txn = app.db().begin().await?;
    // Locked: two editors never overwrite each other's change unseen.
    let current = service::Entity::find_by_id(row.id)
        .lock_exclusive()
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    if let Some(revision) = &body.revision
        && *revision != env_revision(&app, &current)
    {
        return Err(AppError::Conflict {
            code: "env_stale",
            reason: "stale",
            message: "The environment changed since it was read: reload it",
        });
    }
    service::Entity::update_many()
        .col_expr(
            service::Column::Env,
            sea_orm::sea_query::Expr::value(sealed),
        )
        .col_expr(
            service::Column::UpdatedAt,
            sea_orm::sea_query::Expr::value(chrono::Utc::now()),
        )
        .filter(service::Column::Id.eq(row.id))
        .exec(&txn)
        .await?;
    let entry = AuditEntry::new("services.env.update", "services")
        .key(id.to_string())
        .meta(json!({ "names": env.keys().collect::<Vec<_>>() }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(json!({ "names": env.keys().collect::<Vec<_>>() })))
}

/// One deployment, with its log (a finished one: no stream needed).
pub async fn deployment(
    State(app): State<AppState>,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = deployment::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(summary(&row)))
}

/// A database's connection facts, never its passwords (`/credentials`).
fn database_facts(app: &AppState, row: &service::Model, host: &str) -> Value {
    let Some(engine) = Engine::parse(&row.kind) else {
        return Value::Null;
    };
    let env = service_env(row, app.keyring()).unwrap_or_default();
    let credentials = engine.credentials_of(&env);
    json!({
        "engine": engine.kind(),
        "host": host,
        "port": engine.port(),
        "user": credentials.user,
        "database": credentials.database,
    })
}

/// A database's passwords and internal URL (an audited read, like `/env`).
pub async fn credentials(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let db = app.db();
    let row = service_of(&app, id).await?;
    let engine = Engine::parse(&row.kind).ok_or(AppError::NotFound)?;
    let env = service_env(&row, app.keyring()).map_err(|_| {
        AppError::BadRequest("the credentials cannot be read (sealed with another APP_KEY)".into())
    })?;
    let credentials = engine.credentials_of(&env);
    let host = snapshot_without_env(db, &row).await?.service_name();
    let entry = AuditEntry::new("services.credentials.read", "services").key(id.to_string());
    // No audit, no values.
    let txn = db.begin().await?;
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(json!({
        "password": credentials.password,
        "root_password": credentials.root_password,
        "url": engine.url(&host, &credentials),
    })))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseBody {
    pub database: Option<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub root_password: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewServiceBody {
    pub name: String,
    #[serde(default = "app_kind")]
    pub kind: String,
    pub image: Option<String>,
    pub database: Option<DatabaseBody>,
    /// A Compose service's file.
    pub compose: Option<String>,
}

fn app_kind() -> String {
    "app".to_owned()
}

/// A new service in project `id`: an app (`image`), or a database
/// (`kind`: postgres, mysql, …) with generated credentials. Audited.
pub async fn create_service(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<NewServiceBody>,
) -> AppResult<Json<Value>> {
    // Only into a project the user may see (as the resources' forms).
    if !auth.principal().can("projects.view") {
        return Err(AppError::NotFound);
    }
    // A Compose file runs anything on the host (privileged containers, host
    // directories): the panel's own administrators only.
    if body.kind == "compose"
        && !auth
            .principal()
            .can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION)
    {
        return Err(AppError::Forbidden);
    }
    let txn = app.db().begin().await?;
    let project = project::Entity::find_by_id(id)
        .one(&txn)
        .await?
        .ok_or(AppError::NotFound)?;
    let mut new = match Engine::parse(&body.kind) {
        Some(engine) => NewService::database(body.name.trim(), engine),
        None => NewService::app(body.name.trim(), body.image.clone().unwrap_or_default()),
    };
    new.kind = body.kind.clone();
    if Engine::parse(&body.kind).is_some() {
        new.image = body.image.clone();
    }
    if body.kind == "compose" {
        new.image = None;
        new.compose = body.compose.clone();
    }
    if let Some(db) = body.database {
        new.database = DatabaseOptions {
            database: db.database,
            user: db.user,
            password: db.password,
            root_password: db.root_password,
        };
    }
    let row = creation_service::create(&txn, app.keyring(), &project, new).await?;
    let entry = AuditEntry::new("services.create", "services")
        .key(row.id.to_string())
        .meta(json!({ "project": project.name, "name": row.name, "kind": row.kind }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(
        json!({ "id": row.id, "name": row.name, "kind": row.kind }),
    ))
}

/// A new SSH deploy key for the service's repository (ed25519): its private
/// half sealed, its public half shown (add it to the repository, read-only).
pub async fn deploy_key(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = service_of(&app, id).await?;
    let dir = std::env::temp_dir().join(format!(
        "rdp-key-{}",
        crate::services::database_service::random_string(16).to_lowercase()
    ));
    std::fs::create_dir_all(&dir).map_err(AppError::internal)?;
    let path = dir.join("key");
    let made = tokio::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C"])
        .arg(format!("rdockpanel-{id}"))
        .arg("-f")
        .arg(&path)
        .output()
        .await;
    let keys = made.ok().filter(|o| o.status.success()).and_then(|_| {
        Some((
            std::fs::read_to_string(&path).ok()?,
            std::fs::read_to_string(dir.join("key.pub")).ok()?,
        ))
    });
    let _ = std::fs::remove_dir_all(&dir);
    let (private, public) =
        keys.ok_or_else(|| AppError::internal("ssh-keygen could not make a key"))?;
    let public = public.trim().to_owned();
    let sealed = Encrypted::seal(
        app.keyring(),
        &service::deploy_key_context(),
        &private.trim().to_owned(),
    )?;
    let txn = app.db().begin().await?;
    service::Entity::update_many()
        .col_expr(
            service::Column::DeployKey,
            sea_orm::sea_query::Expr::value(Some(sealed)),
        )
        .col_expr(
            service::Column::DeployKeyPublic,
            sea_orm::sea_query::Expr::value(Some(public.clone())),
        )
        .filter(service::Column::Id.eq(row.id))
        .exec(&txn)
        .await?;
    let entry = AuditEntry::new("services.deploy_key", "services").key(id.to_string());
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(json!({ "public_key": public })))
}

/// A new deployment trigger URL (the old one stops working); shown once.
pub async fn trigger(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = service_of(&app, id).await?;
    let token = crate::services::database_service::random_string(40);
    let txn = app.db().begin().await?;
    service::Entity::update_many()
        .col_expr(
            service::Column::TriggerHash,
            sea_orm::sea_query::Expr::value(Some(
                crate::http::controllers::trigger_controller::token_hash(&token),
            )),
        )
        .filter(service::Column::Id.eq(row.id))
        .exec(&txn)
        .await?;
    let entry = AuditEntry::new("services.trigger", "services").key(id.to_string());
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    let base = app.config().app.url.trim_end_matches('/').to_owned();
    Ok(Json(
        json!({ "url": format!("{base}/api/v1/deploy/{id}?token={token}") }),
    ))
}

/// A service's resources now (its Overview's monitor): CPU and memory of each
/// running task on this node, and their sum. Two samples of Docker's stats
/// (the first has no CPU delta), at most 3 s.
pub async fn stats(State(app): State<AppState>, Path(id): Path<i64>) -> AppResult<Json<Value>> {
    use crate::services::docker_service::{Docker, TaskState};
    use futures_util::StreamExt;
    let row = service_of(&app, id).await?;
    let name = snapshot_without_env(app.db(), &row).await?.service_name();
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::BadRequest("Docker is not available".into()))?;
    let tasks = docker
        .engine()
        .list_tasks(&name)
        .await
        .map_err(|e| AppError::BadRequest(format!("the service's tasks: {e}")))?;
    let containers: Vec<(i64, String)> = tasks
        .iter()
        .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
        .filter_map(|t| {
            let container = t
                .status
                .as_ref()?
                .container_status
                .as_ref()?
                .container_id
                .clone()?;
            Some((t.slot.unwrap_or(0), container))
        })
        .collect();
    let samples = futures_util::future::join_all(containers.iter().map(|(slot, container)| {
        let stream = docker.engine().stats(container);
        async move {
            let last = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                stream.take(2).fold(None, |_, s| async move { s.ok() }),
            )
            .await
            .ok()
            .flatten();
            (*slot, last)
        }
    }))
    .await;
    let mut cpu = 0.0;
    let mut mem = 0u64;
    let tasks: Vec<Value> = samples
        .into_iter()
        .filter_map(|(slot, s)| {
            let s = s?;
            cpu += s.cpu_percent;
            mem += s.mem_used;
            Some(json!({ "slot": slot, "cpu_percent": s.cpu_percent, "mem_used": s.mem_used, "mem_limit": s.mem_limit }))
        })
        .collect();
    // Tasks without a sample (stats failed or too slow): said, not counted as 0.
    let missing = containers.len() - tasks.len();
    Ok(Json(
        json!({ "cpu_percent": cpu, "mem_used": mem, "tasks": tasks, "missing": missing }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BasicAuthBody {
    pub username: String,
    pub password: String,
}

/// A user of the service's basic authentication (its Security tab): the
/// password bcrypt-hashed, never shown again; the same user replaced.
/// Applied on the next deployment. Audited.
pub async fn add_basic_auth(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<BasicAuthBody>,
) -> AppResult<Json<Value>> {
    use crate::models::service_basic_auth;
    let row = service_of(&app, id).await?;
    let username = body.username.trim().to_owned();
    let mut errors = ValidationErrors::new();
    if !crate::services::deploy_service::valid_auth_user(&username) {
        errors.add("username", "must be 1-64 letters, digits, _ - . @");
    }
    if body.password.len() < 8 || body.password.len() > 72 {
        errors.add("password", "must be 8-72 characters");
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }
    let password = body.password.clone();
    let hash = tokio::task::spawn_blocking(move || bcrypt::hash(password, 10))
        .await
        .map_err(AppError::internal)?
        .map_err(AppError::internal)?;
    let txn = app.db().begin().await?;
    service_basic_auth::Entity::delete_many()
        .filter(service_basic_auth::Column::ServiceId.eq(row.id))
        .filter(service_basic_auth::Column::Username.eq(username.as_str()))
        .exec(&txn)
        .await?;
    let now = chrono::Utc::now();
    sea_orm::ActiveModelTrait::insert(
        service_basic_auth::ActiveModel {
            service_id: sea_orm::Set(row.id),
            username: sea_orm::Set(username.clone()),
            password_hash: sea_orm::Set(hash),
            created_at: sea_orm::Set(now),
            updated_at: sea_orm::Set(now),
            ..Default::default()
        },
        &txn,
    )
    .await?;
    let entry = AuditEntry::new("services.basic_auth.set", "services")
        .key(id.to_string())
        .meta(json!({ "username": username }));
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(Json(json!({ "username": username })))
}

/// Removes a user of the service's basic authentication. Audited.
pub async fn remove_basic_auth(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path((id, user)): Path<(i64, i64)>,
) -> AppResult<StatusCode> {
    use crate::models::service_basic_auth;
    let row = service_of(&app, id).await?;
    let txn = app.db().begin().await?;
    let removed = service_basic_auth::Entity::delete_many()
        .filter(service_basic_auth::Column::ServiceId.eq(row.id))
        .filter(service_basic_auth::Column::Id.eq(user))
        .exec(&txn)
        .await?;
    if removed.rows_affected == 0 {
        return Err(AppError::NotFound);
    }
    let entry = AuditEntry::new("services.basic_auth.remove", "services").key(id.to_string());
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A new GitHub webhook for the service (auto deploy on a push to its
/// branch): its URL and secret, shown once (the old one stops working).
/// Audited.
pub async fn github_webhook(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let row = service_of(&app, id).await?;
    let secret = crate::services::database_service::random_string(40);
    let sealed = Encrypted::seal(app.keyring(), &service::webhook_secret_context(), &secret)?;
    let txn = app.db().begin().await?;
    service::Entity::update_many()
        .col_expr(
            service::Column::WebhookSecret,
            sea_orm::sea_query::Expr::value(Some(sealed)),
        )
        .filter(service::Column::Id.eq(row.id))
        .exec(&txn)
        .await?;
    let entry = AuditEntry::new("services.github_webhook", "services").key(id.to_string());
    audit::record(&txn, &audit_context(&auth, request), entry).await?;
    txn.commit().await?;
    let base = app.config().app.url.trim_end_matches('/').to_owned();
    Ok(Json(json!({
        "url": format!("{base}/api/v1/github/{id}"),
        "secret": secret,
    })))
}
