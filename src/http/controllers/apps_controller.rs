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

/// The projects the caller sees (every one with a role's `projects.view_any`,
/// else theirs) with their services, for the dashboard: each kind of record
/// only with its permission there (services, domains), and what the caller
/// may do in each.
pub async fn overview(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    use crate::services::access_service;
    let db = app.db();
    let principal = auth.principal();
    let visible = access_service::projects_where(db, principal, "projects.view_any").await?;
    let projects: Vec<project::Model> = project::Entity::find()
        .order_by_asc(project::Column::Name)
        .all(db)
        .await?
        .into_iter()
        .filter(|p| visible.as_ref().is_none_or(|ids| ids.contains(&p.id)))
        .collect();
    let ids: Vec<i64> = projects.iter().map(|p| p.id).collect();
    let services = service::Entity::find()
        .filter(service::Column::ProjectId.is_in(ids.clone()))
        .order_by_asc(service::Column::Name)
        .all(db)
        .await?;
    let domains = service_domain::Entity::find()
        .filter(service_domain::Column::ServiceId.is_in(services.iter().map(|s| s.id)))
        .order_by_asc(service_domain::Column::Id)
        .all(db)
        .await?;
    let mut any_services = false;
    let mut shown = vec![];
    for p in &projects {
        let may = access_service::checker(db, principal, p.id).await?;
        let see_services = may.can("services.view_any");
        let see_domains = may.can("service_domains.view_any");
        any_services |= see_services;
        let services: Vec<Value> = services
            .iter()
            .filter(|s| see_services && s.project_id == p.id)
            .map(|s| {
                let own: Vec<service_domain::Model> = domains
                    .iter()
                    .filter(|d| see_domains && d.service_id == s.id)
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
        shown.push(json!({
            "id": p.id,
            "name": p.name,
            "description": p.description,
            "services": services,
            "role": may.role(),
            "can": {
                "create_service": may.can("services.create"),
                "deploy": may.can("services.deploy"),
                "update": may.can("projects.update"),
                "delete": may.can("projects.delete"),
                "members": may.can("projects.members"),
            },
        }));
    }
    Ok(Json(json!({
        "projects": shown,
        "services_visible": any_services,
        "can": { "create_project": principal.can("projects.create") },
    })))
}

/// A request's body, read once the caller is allowed (a refusal says
/// nothing of it).
pub(crate) fn read_body<T: serde::de::DeserializeOwned>(value: Value) -> AppResult<T> {
    serde_json::from_value(value).map_err(|e| AppError::BadRequest(format!("the request: {e}")))
}

/// The service `id` for `permission`: a role's, or the caller's role in
/// its project (`access_service`).
pub(crate) async fn service_for(
    app: &AppState,
    auth: &Auth,
    id: i64,
    permission: &str,
) -> AppResult<service::Model> {
    crate::services::access_service::service(app.db(), auth.principal(), id, permission).await
}

/// A service's page: its configuration (no secret value), domains, ports,
/// mounts, state and recent deployments.
/// A service with added capabilities, sysctls or groups (or a Compose
/// service): what it runs or how is the panel's administrators' only.
/// [`forbid_privileged`] on the service locked in `txn`: what is checked
/// stays so until the write commits.
async fn lock_unprivileged(
    txn: &sea_orm::DatabaseTransaction,
    auth: &Auth,
    id: i64,
) -> AppResult<()> {
    let row = service::Entity::find_by_id(id)
        .lock_exclusive()
        .one(txn)
        .await?
        .ok_or(AppError::NotFound)?;
    forbid_privileged(auth, &row)
}

pub(crate) fn forbid_privileged(auth: &Auth, row: &service::Model) -> AppResult<()> {
    forbid_privileged_of(auth.principal(), row)
}

/// [`forbid_privileged`] for a principal (a live session's).
pub(crate) fn forbid_privileged_of(
    principal: &rustcms::Principal,
    row: &service::Model,
) -> AppResult<()> {
    if (row.kind == "compose" || crate::admin::resources::service_resource::privileged(row))
        && !principal.can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION)
    {
        return Err(AppError::Forbidden);
    }
    Ok(())
}

pub async fn service(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    let db = app.db();
    let row = service_for(&app, &auth, id, "services.view").await?;
    let principal = auth.principal();
    // Its project's permissions: the caller's roles', or their role's there.
    let may = crate::services::access_service::checker(db, principal, row.project_id).await?;
    let project = project::Entity::find_by_id(row.project_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    let snapshot = snapshot_without_env(db, &row).await?;
    // Related records, each with its own resource's permission.
    let domains = if may.can("service_domains.view_any") {
        service_domain::Entity::find()
            .filter(service_domain::Column::ServiceId.eq(id))
            .order_by_asc(service_domain::Column::Id)
            .all(db)
            .await?
    } else {
        vec![]
    };
    let redirects = if may.can("service_redirects.view_any") {
        crate::models::service_redirect::Entity::find()
            .filter(crate::models::service_redirect::Column::ServiceId.eq(id))
            .order_by_asc(crate::models::service_redirect::Column::Id)
            .all(db)
            .await?
    } else {
        vec![]
    };
    let scripts = if may.can("service_scripts.view_any") {
        crate::models::service_script::Entity::find()
            .filter(crate::models::service_script::Column::ServiceId.eq(id))
            .order_by_asc(crate::models::service_script::Column::Name)
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
    let ports = if may.can("service_ports.view_any") {
        service_port::Entity::find()
            .filter(service_port::Column::ServiceId.eq(id))
            .order_by_asc(service_port::Column::Id)
            .all(db)
            .await?
    } else {
        vec![]
    };
    let mounts = if may.can("service_mounts.view_any") {
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
    let can_deployments = may.can("deployments.view");
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
    let cfg = crate::services::deploy_rules_service::config();
    let default_resolver = (!cfg.cert_resolver.is_empty()).then(|| cfg.cert_resolver.clone());
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
            "git_provider_id": row.git_provider_id,
            "builder": row.builder,
            "dockerfile_path": row.dockerfile_path,
            "dockerfile": row.dockerfile,
            "compose": row.compose,
            "notes": row.notes,
            "cap_add": row.cap_add,
            "cap_drop": row.cap_drop,
            "sysctls": row.sysctls,
            "groups": row.groups,
            "maintenance": row.maintenance,
            "maintenance_title": row.maintenance_title,
            "maintenance_subtitle": row.maintenance_subtitle,
            "rate_limit_average": row.rate_limit_average,
            "rate_limit_period": row.rate_limit_period,
            "rate_limit_burst": row.rate_limit_burst,
            "rate_limit_exempt": row.rate_limit_exempt,
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
        "scripts": scripts,
        "basic_auth": auth_users,
        "ports": ports,
        "mounts": mounts,
        "latest": latest,
        "deployments": deployments,
        // The certificate resolvers a domain may choose (the panel's administrators').
        "cert_resolvers": if principal.can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION) {
            use sea_orm::QueryOrder;
            crate::models::cert_resolver::Entity::find()
                .order_by_asc(crate::models::cert_resolver::Column::Name)
                .all(db)
                .await?
                .into_iter()
                .map(|r| json!({ "id": r.id, "name": r.name, "kind": r.kind }))
                .collect::<Vec<_>>()
        } else {
            vec![]
        },
        // The uploaded certificates a domain may choose (the panel's administrators').
        "certificates": if principal.can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION) {
            use sea_orm::QueryOrder;
            crate::models::certificate::Entity::find()
                .order_by_asc(crate::models::certificate::Column::Name)
                .all(db)
                .await?
                .into_iter()
                .map(|c| json!({ "id": c.id, "name": c.name, "names": c.name_list() }))
                .collect::<Vec<_>>()
        } else {
            vec![]
        },
        // HTTPS without a certificate of its own: the installer's resolver
        // (none: Traefik's own certificate, as in development).
        "default_resolver": default_resolver,
        // Its project's permissions the caller holds (a role's, or their
        // role's there): what its tabs offer.
        "permissions": crate::services::access_service::project_permissions()
            .chain(["docker.exec", "docker.logs"])
            .filter(|p| may.can(p))
            .collect::<Vec<_>>(),
        "can": {
            "update": may.can("services.update"),
            "delete": may.can("services.delete"),
            "deploy": may.can("services.deploy"),
            "destroy": may.can("services.destroy"),
            "secrets": may.can(SECRETS_PERMISSION),
            // The service's logs and terminal (`/apps/services/{id}/…`).
            "logs": may.can("services.logs") || may.can("docker.logs"),
            "exec": may.can("services.exec") || may.can("docker.exec"),
            "members": may.can("projects.members"),
            "deployments": can_deployments,
            "system": principal.can(crate::http::controllers::docker_controller::SYSTEM_PERMISSION),
            "domains": {
                "view": may.can("service_domains.view_any"),
                "create": may.can("service_domains.create"),
                "update": may.can("service_domains.update"),
                "delete": may.can("service_domains.delete"),
            },
            "ports": {
                "view": may.can("service_ports.view_any"),
                "create": may.can("service_ports.create"),
                "delete": may.can("service_ports.delete"),
            },
            "mounts": {
                "view": may.can("service_mounts.view_any"),
                "create": may.can("service_mounts.create"),
                "delete": may.can("service_mounts.delete"),
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
    let row = service_for(&app, &auth, id, "services.secrets").await?;
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
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    let row = service_for(&app, &auth, id, "services.update").await?;
    let body: EnvText = read_body(body)?;
    // A Compose file reads its environment (`${VAR}`, even in its volumes):
    // the panel's administrators only, as the file itself.
    if (row.kind == "compose" || crate::admin::resources::service_resource::privileged(&row))
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
    // Checked again on the locked row (it may have become privileged).
    forbid_privileged(&auth, &current)?;
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
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    if !crate::services::access_service::admits(app.db(), auth.principal(), "deployments.view")
        .await?
    {
        return Err(AppError::Forbidden);
    }
    let row = deployment::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    service_for(&app, &auth, row.service_id, "deployments.view").await?;
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
    let row = service_for(&app, &auth, id, "services.secrets").await?;
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
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    // Into a project where the user may add services (a role's, or theirs).
    crate::services::access_service::project(app.db(), auth.principal(), id, "services.create")
        .await?;
    let body: NewServiceBody = read_body(body)?;
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
    let row = service_for(&app, &auth, id, "services.update").await?;
    forbid_privileged(&auth, &row)?;
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
    let txn = rustcms::db::begin_write(app.db()).await?;
    lock_unprivileged(&txn, &auth, row.id).await?;
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
    let row = service_for(&app, &auth, id, "services.deploy").await?;
    forbid_privileged(&auth, &row)?;
    let token = crate::services::database_service::random_string(40);
    let txn = rustcms::db::begin_write(app.db()).await?;
    lock_unprivileged(&txn, &auth, row.id).await?;
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
/// How many task errors and how much of a failed deployment's log the
/// Overview shows.
const RECENT_ERRORS: usize = 10;
const FAILED_LOG_BYTES: usize = 4_096;

/// The service's recent errors, like EasyPanel's Overview: its tasks that
/// failed (an exit code other than 0, refused by Swarm, lost with their
/// node; not those stopped by a deployment), and its last failed deployment
/// with the end of its log (`deployments.view`).
pub async fn errors(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    use crate::services::docker_service::{Docker, TaskState};
    let row = service_for(&app, &auth, id, "services.view").await?;
    // A Compose or privileged service's diagnostics: its administrators'.
    forbid_privileged(&auth, &row)?;
    let snapshot = snapshot_without_env(app.db(), &row).await?;
    let name = snapshot.service_name();
    // Compose's containers are no Swarm tasks: none listed. Only the panel's
    // own deployment of it (never another Swarm service of that name).
    let tasks = match app.service::<Docker>() {
        Some(docker)
            if row.kind != "compose"
                && matches!(
                    docker
                        .managed_service(&name, &snapshot.project, &snapshot.service)
                        .await,
                    Ok(Some(_))
                ) =>
        {
            docker.engine().list_tasks(&name).await.unwrap_or_default()
        }
        _ => vec![],
    };
    // Newest first (by time, not by text: precisions vary).
    let mut tasks = tasks;
    tasks.sort_by(|a, b| {
        let at = |t: &crate::services::docker_service::SwarmTask| {
            t.status.as_ref().and_then(|s| s.timestamp)
        };
        at(b).cmp(&at(a))
    });
    // Running now in a slot (since when): a failure there before is over.
    let running: Vec<(Option<i64>, _)> = tasks
        .iter()
        .filter_map(|t| {
            let status = t.status.as_ref()?;
            (status.state == Some(TaskState::RUNNING)).then_some((t.slot, status.timestamp))
        })
        .collect();
    let mut failed: Vec<Value> = tasks
        .iter()
        .filter_map(|task| {
            let status = task.status.as_ref()?;
            let state = status.state?;
            if !matches!(
                state,
                TaskState::FAILED | TaskState::REJECTED | TaskState::ORPHANED
            ) {
                return None;
            }
            let error: String = status
                .err
                .clone()
                .or_else(|| status.message.clone())
                .unwrap_or_default()
                .chars()
                .take(500)
                .collect();
            Some(json!({
                "at": status.timestamp,
                "slot": task.slot,
                "state": state.to_string(),
                "error": error,
                "exit_code": status.container_status.as_ref().and_then(|c| c.exit_code),
                // Swarm replaced it, and that one runs.
                "recovered": running.iter().any(|(slot, since)| {
                    (task.slot.is_none() || *slot == task.slot) && *since > status.timestamp
                }),
            }))
        })
        .collect();
    failed.truncate(RECENT_ERRORS);
    let deployment = if crate::services::access_service::can(
        app.db(),
        auth.principal(),
        row.project_id,
        "deployments.view",
    )
    .await?
    {
        deployment::Entity::find()
            .filter(deployment::Column::ServiceId.eq(id))
            .filter(deployment::Column::Status.eq(crate::services::deployment_service::FAILED))
            .order_by_desc(deployment::Column::Id)
            .one(app.db())
            .await?
            .map(|d| {
                let log = d.log.as_deref().unwrap_or_default();
                // Its end, cut at a line (and a character) boundary.
                let mut start = log.len().saturating_sub(FAILED_LOG_BYTES);
                while !log.is_char_boundary(start) {
                    start += 1;
                }
                let tail = &log[start..];
                // From its first whole line, unless that leaves nothing.
                let tail = match tail.split_once('\n') {
                    Some((_, rest)) if start > 0 && !rest.trim().is_empty() => rest,
                    _ => tail,
                };
                json!({
                    "id": d.id,
                    "trigger": d.trigger,
                    "error": d.error,
                    "created_at": d.created_at,
                    "finished_at": d.finished_at,
                    "log_tail": tail,
                })
            })
    } else {
        None
    };
    Ok(Json(json!({ "tasks": failed, "deployment": deployment })))
}

pub async fn stats(
    State(app): State<AppState>,
    auth: Auth,
    Path(id): Path<i64>,
) -> AppResult<Json<Value>> {
    use crate::services::docker_service::{Docker, TaskState};
    use futures_util::StreamExt;
    let row = service_for(&app, &auth, id, "services.view").await?;
    // Container ids (the Logs and Shell buttons) for who sees containers, or
    // this service's logs or terminal.
    let may = crate::services::access_service::checker(app.db(), auth.principal(), row.project_id)
        .await?;
    let show_ids = may.can("docker.view") || may.can("services.logs") || may.can("services.exec");
    let snapshot = snapshot_without_env(app.db(), &row).await?;
    let name = snapshot.service_name();
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::BadRequest("Docker is not available".into()))?;
    // The panel's own deployment of it only (never another Swarm service of
    // that name): none yet, no task.
    let managed = docker
        .managed_service(&name, &snapshot.project, &snapshot.service)
        .await
        .map_err(|e| AppError::BadRequest(format!("the service: {e}")))?
        .is_some();
    let tasks = if managed {
        docker
            .engine()
            .list_tasks(&name)
            .await
            .map_err(|e| AppError::BadRequest(format!("the service's tasks: {e}")))?
    } else {
        vec![]
    };
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
    let ids: std::collections::HashMap<i64, String> = if show_ids {
        containers.iter().cloned().collect()
    } else {
        Default::default()
    };
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
            Some(json!({ "slot": slot, "container": ids.get(&slot), "cpu_percent": s.cpu_percent, "mem_used": s.mem_used, "mem_limit": s.mem_limit }))
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
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    use crate::models::service_basic_auth;
    let row = service_for(&app, &auth, id, "services.update").await?;
    let body: BasicAuthBody = read_body(body)?;
    forbid_privileged(&auth, &row)?;
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
    let txn = rustcms::db::begin_write(app.db()).await?;
    lock_unprivileged(&txn, &auth, row.id).await?;
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
    // Applied without a deployment: its routes written again.
    rustcms::queue::dispatch(
        &txn,
        &crate::jobs::publish_routes::PublishRoutes { service_id: id },
    )
    .await?;
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
    let row = service_for(&app, &auth, id, "services.update").await?;
    forbid_privileged(&auth, &row)?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    lock_unprivileged(&txn, &auth, row.id).await?;
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
    // Applied without a deployment: its routes written again.
    rustcms::queue::dispatch(
        &txn,
        &crate::jobs::publish_routes::PublishRoutes { service_id: id },
    )
    .await?;
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
    let row = service_for(&app, &auth, id, "services.deploy").await?;
    forbid_privileged(&auth, &row)?;
    let secret = crate::services::database_service::random_string(40);
    let sealed = Encrypted::seal(app.keyring(), &service::webhook_secret_context(), &secret)?;
    let txn = rustcms::db::begin_write(app.db()).await?;
    lock_unprivileged(&txn, &auth, row.id).await?;
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunScriptBody {
    /// The command as the one who runs it saw it: run only if it is still so.
    pub command: String,
}

/// How long a script may run (within the request's own timeout).
const SCRIPT_TIME: std::time::Duration = std::time::Duration::from_secs(25);

/// Runs a script of a service in one of its running containers (`sh -c`,
/// at most 25 s, its output's last 64 KB): `{exit_code, output}`. Only the
/// command shown (409 if changed meanwhile), only in a service the panel
/// deployed. Logged as an action (`script.run`: a run cut by a stop is
/// killed at boot) and audited (its name, never its output).
pub async fn run_script(
    State(app): State<AppState>,
    auth: Auth,
    request: RequestMeta,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> AppResult<Json<Value>> {
    use crate::models::service_script;
    use crate::services::action_service::{self, Initiator};
    use crate::services::docker_service::{Docker, ExecChunk, TaskState};
    use futures_util::StreamExt;
    // Nowhere allowed: refused before any id is looked up.
    // Its own permission (a role's, or a project role's), or Docker's.
    if !auth.principal().can("docker.exec")
        && !crate::services::access_service::admits(app.db(), auth.principal(), "services.exec")
            .await?
    {
        return Err(AppError::Forbidden);
    }
    let body: RunScriptBody = read_body(body)?;
    let script = service_script::Entity::find_by_id(id)
        .one(app.db())
        .await?
        .ok_or(AppError::NotFound)?;
    // Its service's project first: its command is compared only then.
    let row = crate::services::access_service::service_any(
        app.db(),
        auth.principal(),
        script.service_id,
        &["services.exec", "docker.exec"],
    )
    .await?;
    if script.command != body.command {
        return Err(AppError::Conflict {
            code: "script_changed",
            reason: "changed",
            message: "The script changed since it was shown: look at it again",
        });
    }
    forbid_privileged(&auth, &row)?;
    if row.kind == "compose" {
        return Err(AppError::BadRequest(
            "a Compose service's scripts are not run by the panel".into(),
        ));
    }
    let snapshot = snapshot_without_env(app.db(), &row).await?;
    let name = snapshot.service_name();
    let docker = app
        .service::<Docker>()
        .ok_or_else(|| AppError::BadRequest("Docker is not available".into()))?;
    // The panel's own deployment of this service (never another object of
    // that name, such as one of the panel's).
    docker
        .managed_service(&name, &snapshot.project, &snapshot.service)
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
        .ok_or_else(|| AppError::BadRequest("the service is not deployed".into()))?;
    let tasks = docker
        .engine()
        .list_tasks(&name)
        .await
        .map_err(|e| AppError::BadRequest(format!("the service's tasks: {e}")))?;
    let container = tasks
        .iter()
        .filter(|t| t.status.as_ref().and_then(|s| s.state) == Some(TaskState::RUNNING))
        .find_map(|t| {
            t.status
                .as_ref()?
                .container_status
                .as_ref()?
                .container_id
                .clone()
        })
        .ok_or_else(|| AppError::BadRequest("the service is not running".into()))?;
    let entry = AuditEntry::new("services.script.run", "services")
        .key(row.id.to_string())
        .meta(json!({ "script": script.name }));
    let txn = app.db().begin().await?;
    audit::record(&txn, &audit_context(&auth, request.clone()), entry).await?;
    txn.commit().await?;
    let mark = format!(
        "rdp-script-{}",
        crate::services::database_service::random_string(20)
    );
    let by = Initiator::user(auth.principal(), request);
    let action = action_service::start(
        app.db(),
        &by,
        SCRIPT_ACTION,
        &container,
        crate::models::action::RUNNING,
    )
    .await?;
    let action = action_service::note(app.db(), action, &format!("{SCRIPT_MARK}{mark}")).await?;
    let cmd = vec!["sh".to_owned(), "-c".to_owned(), script.command.clone()];
    // Run apart from the request: its action is finished even if the
    // request is dropped (its timeout); the response waits for it.
    let db = app.db().clone();
    let run = tokio::spawn(async move {
        let mut chunks = match docker
            .engine()
            .run_in_container(&container, cmd, &mark, None)
            .await
        {
            Ok(chunks) => chunks,
            Err(e) => {
                let _ = action_service::finish(&db, action, Some(e.to_string()), None).await;
                return Err(format!("the script could not start: {e}"));
            }
        };
        let mut output = Vec::new();
        let mut code = None;
        let read = async {
            while let Some(chunk) = chunks.next().await {
                match chunk {
                    Ok(ExecChunk::Out(bytes)) | Ok(ExecChunk::Err(bytes)) => {
                        output.extend_from_slice(&bytes);
                        if output.len() > 128 * 1024 {
                            let cut = output.len() - 64 * 1024;
                            output.drain(..cut);
                        }
                    }
                    Ok(ExecChunk::Exit(c)) => code = Some(c),
                    Err(e) => {
                        output.extend_from_slice(format!("\n{e}\n").as_bytes());
                        break;
                    }
                }
            }
        };
        if tokio::time::timeout(SCRIPT_TIME, read).await.is_err() {
            // Dropped: its processes are killed in the container.
            output.extend_from_slice(b"\n(stopped after 25 s)\n");
        }
        let start = output.len().saturating_sub(64 * 1024);
        let text = String::from_utf8_lossy(&output[start..]).into_owned();
        let error = match code {
            Some(0) => None,
            Some(c) => Some(format!("exited with {c}")),
            None => Some("stopped".to_owned()),
        };
        let _ = action_service::finish(&db, action, error, None).await;
        Ok((code, text))
    });
    let (code, text) = run
        .await
        .map_err(AppError::internal)?
        .map_err(AppError::BadRequest)?;
    Ok(Json(json!({ "exit_code": code, "output": text })))
}

/// A script's run in the actions log; its log holds the mark of its
/// processes (`SCRIPT_MARK<mark>`), to kill them after a stop.
pub const SCRIPT_ACTION: &str = "script.run";
pub const SCRIPT_MARK: &str = "mark:";
