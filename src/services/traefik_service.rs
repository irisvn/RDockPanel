//! Traefik: the routes of deployed services, as files of its file provider,
//! and Traefik's attachment to each project's network (it reaches services
//! there by name).
//!
//! One file per service, `<project>_<service>.yml`, in [`Traefik::dir`]
//! (the panel writes, Traefik reads). A file is written whole: to a hidden
//! temporary file first (outside the `*.yml` Traefik watches), then renamed.
//! At boot every file is written again from the database ([`sync_all`]), and
//! files of services that no longer route are removed.
//!
//! **One writer at a time.** Each write reads what the database says now and
//! writes it under one lock (the panel runs as one process: its workers
//! included), so the last write is always the latest state: a slow publisher
//! never puts back routes a Stop removed after it read them.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rustcms::{AppError, AppResult};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder};

use crate::models::{deployment, service};
use crate::services::deploy_rules_service::{PanelConfig, valid_name};
use crate::services::deploy_service::{DeployOptions, Snapshot, TraefikFile, routes};
use crate::services::docker_service::{Docker, DockerError, DockerResult, NetworkAttachmentConfig};

/// Where the routes go and which Swarm service is Traefik (bound by the app;
/// tests bind one on a directory of their own).
#[derive(Debug, Clone)]
pub struct Traefik {
    /// The file provider's directory.
    pub dir: PathBuf,
    /// Traefik's Swarm service, attached to each project's network; empty:
    /// none (Traefik runs elsewhere and reaches services by itself).
    pub service: String,
    /// Whether [`sync_all`] removes route files of services the database
    /// does not have (the panel owns the directory; tests sharing one don't).
    pub prune: bool,
    /// Held while routes are read from the database and written.
    writing: Arc<tokio::sync::Mutex<()>>,
}

impl Traefik {
    pub fn new(dir: PathBuf, service: impl Into<String>, prune: bool) -> Self {
        Self {
            dir,
            service: service.into(),
            prune,
            writing: Arc::default(),
        }
    }

    pub fn from_config(config: &PanelConfig) -> Self {
        Self::new(
            config.traefik_dir.clone(),
            config.traefik_service.clone(),
            true,
        )
    }

    fn path(&self, snapshot_name: &str) -> PathBuf {
        self.dir.join(format!("{snapshot_name}.yml"))
    }

    /// Removes the routes of the Swarm service `name` (before it stops).
    pub async fn remove(&self, name: &str) -> std::io::Result<()> {
        let _writing = self.writing.lock().await;
        self.write(name, None)
    }

    /// Writes (or, with `None`, removes) the routes of the Swarm service
    /// `name` (`<project>_<service>`, checked names only). Callers hold
    /// [`Self::writing`].
    fn write(&self, name: &str, routes: Option<&TraefikFile>) -> std::io::Result<()> {
        if !is_route_file_name(name) {
            return Err(std::io::Error::other("not a service name"));
        }
        let path = self.path(name);
        match routes {
            Some(routes) => write_atomically(&self.dir, &path, routes.to_yaml().as_bytes()),
            None => match std::fs::remove_file(&path) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
                _ => Ok(()),
            },
        }
    }

    /// Attaches Traefik to `network` (its name and id) unless it is already:
    /// Swarm restarts Traefik then (a few seconds without routing). Whether
    /// it changed anything.
    pub async fn attach(&self, docker: &Docker, network: &str, id: &str) -> DockerResult<bool> {
        if self.service.is_empty() {
            return Ok(false);
        }
        let api = docker.engine();
        for _ in 0..5 {
            let Some(current) = api.inspect_service(&self.service).await? else {
                return Err(DockerError::NotFound);
            };
            let mut spec = current.spec.clone().unwrap_or_default();
            let template = spec.task_template.get_or_insert_with(Default::default);
            let networks = template.networks.get_or_insert_with(Vec::new);
            if networks
                .iter()
                .any(|n| matches!(n.target.as_deref(), Some(t) if t == network || t == id))
            {
                return Ok(false);
            }
            networks.push(NetworkAttachmentConfig {
                target: Some(network.to_owned()),
                ..Default::default()
            });
            let version = current.version.and_then(|v| v.index).unwrap_or(0);
            match api.update_service(&self.service, version, spec, None).await {
                Err(DockerError::Conflict(_)) => {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                other => return other.map(|_| true),
            }
        }
        Err(DockerError::Conflict("Traefik kept changing".into()))
    }
}

impl Traefik {
    /// Detaches Traefik from `network` (its name or id) when it is attached:
    /// whether it changed anything.
    pub async fn detach(&self, docker: &Docker, network: &str, id: &str) -> DockerResult<bool> {
        if self.service.is_empty() {
            return Ok(false);
        }
        let api = docker.engine();
        for _ in 0..5 {
            let Some(current) = api.inspect_service(&self.service).await? else {
                return Ok(false);
            };
            let mut spec = current.spec.clone().unwrap_or_default();
            let template = spec.task_template.get_or_insert_with(Default::default);
            let networks = template.networks.get_or_insert_with(Vec::new);
            let before = networks.len();
            networks.retain(|n| !matches!(n.target.as_deref(), Some(t) if t == network || t == id));
            if networks.len() == before {
                return Ok(false);
            }
            let version = current.version.and_then(|v| v.index).unwrap_or(0);
            match api.update_service(&self.service, version, spec, None).await {
                Err(DockerError::Conflict(_)) => {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                other => return other.map(|_| true),
            }
        }
        Err(DockerError::Conflict("Traefik kept changing".into()))
    }
}

/// The networks of projects deleted (labelled by the panel, their project
/// gone): Traefik detached, the network removed. A network still in use is
/// left for the next time. How many were removed.
pub async fn remove_orphan_networks(
    db: &sea_orm::DatabaseConnection,
    docker: &Docker,
    traefik: Option<&Traefik>,
) -> rustcms::AppResult<usize> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    let projects: std::collections::HashSet<String> = crate::models::project::Entity::find()
        .all(db)
        .await?
        .into_iter()
        .map(|p| p.name)
        .collect();
    let networks = match docker.engine().networks().await {
        Ok(networks) => networks,
        Err(err) => {
            tracing::debug!(%err, "networks not listed");
            return Ok(0);
        }
    };
    let mut removed = 0;
    for network in networks {
        let ours = network
            .labels
            .get(crate::services::docker_service::MANAGED_LABEL)
            .is_some_and(|v| v == "true");
        let Some(project) = network.labels.get("rdockpanel.project") else {
            continue;
        };
        if !ours
            || projects.contains(project)
            || network.name != crate::services::deploy_service::network_name(project)
        {
            continue;
        }
        // Checked again just before (a project made meanwhile keeps it).
        if crate::models::project::Entity::find()
            .filter(crate::models::project::Column::Name.eq(project.as_str()))
            .one(db)
            .await?
            .is_some()
        {
            continue;
        }
        if let Some(traefik) = traefik
            && let Err(err) = traefik.detach(docker, &network.name, &network.id).await
        {
            tracing::warn!(%err, network = network.name, "Traefik not detached");
            continue;
        }
        match docker.engine().remove_network(&network.id).await {
            Ok(()) => {
                tracing::info!(
                    network = network.name,
                    "removed the network of a deleted project"
                );
                removed += 1;
            }
            Err(err) => tracing::debug!(%err, network = network.name, "network not removed yet"),
        }
    }
    Ok(removed)
}

/// `<project>_<service>`: two checked names (no path, no dot).
fn is_route_file_name(name: &str) -> bool {
    name.split_once('_')
        .is_some_and(|(project, service)| valid_name(project) && valid_name(service))
}

fn write_atomically(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| std::io::Error::other("no file name"))?;
    // A name of its own (never another writer's half-written file).
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let temporary = dir.join(format!(".{name}.{}.{n}.tmp", std::process::id()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// The routes a service has now: those of the deployment that made it what
/// it is (its configuration hash, kept when a later deployment failed), when
/// that one runs it. `None` when it does not route (never deployed, stopped,
/// destroyed, no domain).
pub async fn deployed_routes(
    db: &DatabaseConnection,
    row: &service::Model,
    options: &DeployOptions,
) -> AppResult<Option<TraefikFile>> {
    let Some(hash) = row.deployed_spec_hash.as_deref() else {
        return Ok(None);
    };
    let Some(deployed) = deployment::Entity::find()
        .filter(deployment::Column::ServiceId.eq(row.id))
        .filter(deployment::Column::SpecHash.eq(hash))
        .filter(deployment::Column::Status.eq(crate::services::deployment_service::SUCCEEDED))
        .order_by_desc(deployment::Column::Id)
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    let mut snapshot: Snapshot =
        serde_json::from_value(deployed.spec).map_err(AppError::internal)?;
    // Its basic authentication's users, with their hashes now (never kept
    // in a deployment).
    if !snapshot.basic_auth.is_empty() {
        // Exactly the users deployed (name and password): one changed
        // since signs in with the new password from the next deployment.
        let deployed: std::collections::HashSet<String> =
            snapshot.basic_auth.iter().cloned().collect();
        let mut users: Vec<String> = crate::models::service_basic_auth::Entity::find()
            .filter(crate::models::service_basic_auth::Column::ServiceId.eq(row.id))
            .all(db)
            .await?
            .into_iter()
            .filter(|u| {
                deployed.contains(&crate::services::deploy_service::auth_digest(
                    &u.username,
                    &u.password_hash,
                ))
            })
            .map(|u| format!("{}:{}", u.username, u.password_hash))
            .collect();
        users.sort();
        // Every user removed since: nobody gets in (never open by accident).
        if users.is_empty() {
            users.push("nobody:!".to_owned());
        }
        snapshot.basic_auth = users;
    }
    Ok(snapshot
        .running
        .then(|| routes(&snapshot, options))
        .flatten())
}

/// The Swarm name of a service row (its project's name read).
pub async fn swarm_name(db: &DatabaseConnection, row: &service::Model) -> AppResult<String> {
    let project = crate::models::project::Entity::find_by_id(row.project_id)
        .one(db)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(crate::services::deploy_service::service_name(
        &project.name,
        &row.name,
    ))
}

/// Writes the routes of `row` as deployed now (removes them when it does
/// not route). Whether they were written.
pub async fn republish(
    db: &DatabaseConnection,
    traefik: &Traefik,
    row: &service::Model,
    options: &DeployOptions,
) -> AppResult<bool> {
    let _writing = traefik.writing.lock().await;
    // Read again under the lock: what the database says now.
    let Some(row) = service::Entity::find_by_id(row.id).one(db).await? else {
        return Ok(true);
    };
    let routes = deployed_routes(db, &row, options).await?;
    let name = swarm_name(db, &row).await?;
    match traefik.write(&name, routes.as_ref()) {
        Ok(()) => Ok(true),
        Err(err) => {
            tracing::warn!(%err, service = name, "routes not written");
            Ok(false)
        }
    }
}

/// At boot: every service's routes written again, and route files of
/// services that no longer exist or route removed. How many were written.
pub async fn sync_all(
    db: &DatabaseConnection,
    traefik: &Traefik,
    options: &DeployOptions,
) -> AppResult<usize> {
    // Nothing is published meanwhile (the list below stays the truth).
    let _writing = traefik.writing.lock().await;
    let mut keep = std::collections::BTreeSet::new();
    let mut written = 0;
    for row in service::Entity::find().all(db).await? {
        let name = swarm_name(db, &row).await?;
        let routes = deployed_routes(db, &row, options).await?;
        if routes.is_some() {
            keep.insert(format!("{name}.yml"));
        }
        if traefik.write(&name, routes.as_ref()).is_ok() && routes.is_some() {
            written += 1;
        }
    }
    // Files left by services since renamed or deleted.
    if !traefik.prune {
        return Ok(written);
    }
    if let Ok(entries) = std::fs::read_dir(&traefik.dir) {
        for entry in entries.flatten() {
            let file = entry.file_name().to_string_lossy().into_owned();
            let ours = file.strip_suffix(".yml").is_some_and(is_route_file_name);
            if ours && !keep.contains(&file) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_files_are_service_names_only() {
        assert!(is_route_file_name("shop_web"));
        for bad in [
            "shop",
            "../x_web",
            "shop_web.yml",
            "Shop_web",
            "shop_we/b",
            "_web",
        ] {
            assert!(!is_route_file_name(bad), "{bad}");
        }
    }

    #[test]
    fn files_are_written_whole_and_removed() {
        let dir = std::env::temp_dir().join(format!("rdp-traefik-{}", std::process::id()));
        let traefik = Traefik::new(dir.clone(), "", true);
        let routes = TraefikFile {
            http: crate::services::deploy_service::TraefikHttp {
                routers: Default::default(),
                services: Default::default(),
                middlewares: Default::default(),
            },
        };
        traefik.write("shop_web", Some(&routes)).unwrap();
        assert!(dir.join("shop_web.yml").is_file());
        let leftovers = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary file left");
        traefik.write("shop_web", None).unwrap();
        traefik.write("shop_web", None).unwrap();
        assert!(!dir.join("shop_web.yml").exists());
        assert!(traefik.write("../etc_x", Some(&routes)).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
