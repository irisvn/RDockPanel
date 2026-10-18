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
                        "kind": "app",
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
            "kind": "app",
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
        "env_readable": env_readable,
        "domains": domains,
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
