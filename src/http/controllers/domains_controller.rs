//! The Domains page (`/admin/api/x/apps/domains`), like EasyPanel's: every
//! service's domains with where they lead, and whether Traefik serves them
//! as configured — a domain added or changed is served from the service's
//! next deployment, a removed one until then. Writes go through the
//! domains' resource (its policy and validation).

use std::collections::HashMap;

use rustcms::prelude::*;
use rustcms::{AppError, AppResult, Json};
use sea_orm::{EntityTrait, QueryOrder};
use serde_json::Value;

use crate::http::controllers::apps_controller::forbid_privileged;
use crate::http::controllers::docker_controller::SYSTEM_PERMISSION;
use crate::models::{project, service, service_domain};
use crate::services::deploy_service::{Domain, Snapshot, compose_alias};
use crate::services::deployment_service::snapshot_without_env;
use crate::services::traefik_service::deployed_snapshot;

/// Served as configured.
const LIVE: &str = "live";
/// Added or changed: served so from the next deployment.
const PENDING: &str = "pending";
/// Removed, still served until the next deployment.
const REMOVED: &str = "removed";
/// Its service does not run (stopped, never deployed): not served.
const STOPPED: &str = "stopped";

pub async fn index(State(app): State<AppState>, auth: Auth) -> AppResult<Json<Value>> {
    let db = app.db();
    let principal = auth.principal();
    use crate::services::access_service::{admits, checker};
    // Where a domain leads names its service: both, somewhere.
    if !admits(db, principal, "services.view_any").await?
        || !admits(db, principal, "service_domains.view_any").await?
    {
        return Err(AppError::Forbidden);
    }
    let admin = principal.can(SYSTEM_PERMISSION);
    let projects: HashMap<i64, project::Model> = project::Entity::find()
        .all(db)
        .await?
        .into_iter()
        .map(|p| (p.id, p))
        .collect();
    let services = service::Entity::find()
        .order_by_asc(service::Column::Name)
        .all(db)
        .await?;
    let mut rows: HashMap<i64, Vec<service_domain::Model>> = HashMap::new();
    for row in service_domain::Entity::find()
        .order_by_asc(service_domain::Column::Id)
        .all(db)
        .await?
    {
        rows.entry(row.service_id).or_default().push(row);
    }
    let mut domains: Vec<Value> = vec![];
    let mut choices: Vec<Value> = vec![];
    for row in &services {
        let Some(project) = projects.get(&row.project_id) else {
            continue;
        };
        // Its project's: the caller's roles', or their role's there.
        let may = checker(db, principal, project.id).await?;
        if !may.can("services.view_any") || !may.can("service_domains.view_any") {
            continue;
        }
        // A Compose or privileged service's records are the panel's
        // administrators' (`ChildPolicy`).
        let unprivileged = forbid_privileged(&auth, row).is_ok();
        let writable = unprivileged && may.can("service_domains.update");
        choices.push(json!({
            "id": row.id,
            "project": project.name,
            "name": row.name,
            "kind": row.kind,
            "writable": unprivileged && may.can("service_domains.create"),
        }));
        let own = rows.remove(&row.id).unwrap_or_default();
        let current = snapshot_without_env(db, row).await?;
        let deployed = deployed_snapshot(db, row).await?;
        if own.is_empty() && deployed.as_ref().is_none_or(|d| d.domains.is_empty()) {
            continue;
        }
        let base = current.service_name();
        let at = |d: &Domain| (d.host.clone(), d.path.clone());
        let configured: HashMap<(String, String), &Domain> =
            current.domains.iter().map(|d| (at(d), d)).collect();
        let served: HashMap<(String, String), &Domain> = deployed
            .as_ref()
            .filter(|d| d.running)
            .map(|d| d.domains.iter().map(|x| (at(x), x)).collect())
            .unwrap_or_default();
        let running = deployed.as_ref().is_some_and(|d| d.running);
        // What every route of the service carries: changed, each waits.
        let access = deployed.as_ref().is_some_and(|d| {
            d.maintenance == current.maintenance
                && d.redirects == current.redirects
                && d.basic_auth == current.basic_auth
        });
        // In maintenance, Traefik sends its domains to the panel's page.
        let maintenance = deployed
            .as_ref()
            .is_some_and(|d| d.running && d.maintenance);
        for (i, domain) in own.iter().enumerate() {
            let key = (domain.host.clone(), domain.path.clone());
            let status = match (configured.get(&key), served.get(&key)) {
                _ if !running => STOPPED,
                (Some(now), Some(then)) if now == then && access => LIVE,
                _ => PENDING,
            };
            domains.push(json!({
                "id": domain.id,
                "project": project.name,
                "service": row.name,
                "service_id": row.id,
                "kind": row.kind,
                "primary": i == 0,
                "host": domain.host,
                "path": domain.path,
                "port": domain.port,
                "https": domain.https,
                "compose_service": domain.compose_service,
                "middlewares": domain.middlewares,
                "cert_resolver_id": domain.cert_resolver_id,
                "certificate_id": domain.certificate_id,
                "target": target(&base, &current, domain.compose_service.as_deref(), domain.port),
                "maintenance": maintenance,
                "status": status,
                "writable": writable,
            }));
        }
        // Removed since its deployment: Traefik serves them still.
        for (key, gone) in &served {
            if configured.contains_key(key) {
                continue;
            }
            let deployed = deployed.as_ref().expect("served from a deployment");
            domains.push(json!({
                "id": Value::Null,
                "project": project.name,
                "service": row.name,
                "service_id": row.id,
                "kind": row.kind,
                "primary": false,
                "host": gone.host,
                "path": gone.path,
                "port": gone.port,
                "https": gone.https,
                "compose_service": gone.compose_service,
                "target": target(&base, deployed, gone.compose_service.as_deref(), i32::from(gone.port)),
                "maintenance": maintenance,
                "status": REMOVED,
                "writable": false,
            }));
        }
    }
    // The panel's own (the installer's), for its administrators only.
    let cfg = crate::services::deploy_rules_service::config();
    let panel = (admin
        && !cfg.panel_host.is_empty()
        && cfg.panel_host != "localhost"
        && cfg.panel_host.parse::<std::net::IpAddr>().is_err())
    .then(|| json!({ "host": cfg.panel_host, "url": cfg.panel_url }));
    Ok(Json(json!({
        "domains": domains,
        "panel": panel,
        "services": choices,
        // HTTPS without a certificate of its own: the installer's resolver
        // (none: Traefik's own certificate, as in development).
        "default_resolver": (!cfg.cert_resolver.is_empty()).then(|| cfg.cert_resolver.clone()),
        // What a domain may choose (the panel's administrators').
        "cert_resolvers": if admin {
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
        "certificates": if admin {
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
        "can": {
            // Somewhere (each row and service says where).
            "create": admits(db, principal, "service_domains.create").await?,
            "update": admits(db, principal, "service_domains.update").await?,
            "delete": admits(db, principal, "service_domains.delete").await?,
            "system": admin,
        },
    })))
}

/// Where Traefik sends a domain: the service on its project's network (a
/// Compose service's container), as EasyPanel shows it.
fn target(base: &str, snapshot: &Snapshot, container: Option<&str>, port: i32) -> String {
    let host = match (&snapshot.compose, container) {
        (Some(_), Some(sub)) => compose_alias(base, sub),
        _ => base.to_owned(),
    };
    format!("http://{host}:{port}/")
}
