//! `services` table: an app deployed as a Docker Swarm service
//! (`<project>_<service>`). What is configured here becomes the service on
//! the next deployment; `deployed_at` marks the first one. Once its status
//! is no longer [`NEW`] (deploying, deployed, or left by a failed first run),
//! the name and the project are fixed and deleting needs Destroy first.

use std::collections::BTreeMap;

use rustcms::crypto::{Context, Encrypted};
use sea_orm::entity::prelude::*;

/// The status of a service Docker has never had (or no longer has: destroyed).
pub const NEW: &str = "new";

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "services")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    /// `app`, a database (`postgres`, `mysql`, `mariadb`, `mongo`, `redis`),
    /// or `compose` (containers of a Compose file, run by `docker compose`).
    pub kind: String,
    pub image: String,
    /// Where the image comes from: `image`, `git` (built), `dockerfile`
    /// (written here, built).
    pub source: String,
    pub git_url: Option<String>,
    /// The branch, tag or commit built (`main` by default).
    pub git_branch: Option<String>,
    /// The build context in the repository (`/` by default).
    pub git_path: Option<String>,
    /// `dockerfile` (the repository's) or `nixpacks`.
    pub builder: String,
    /// The Dockerfile's path in the context (`Dockerfile` by default).
    pub dockerfile_path: Option<String>,
    /// A Dockerfile written in the panel (`source = dockerfile`).
    #[sea_orm(column_type = "Text", nullable)]
    pub dockerfile: Option<String>,
    /// The SSH deploy key's private half, sealed (context `services.deploy_key`).
    #[sea_orm(column_type = "Text", nullable)]
    pub deploy_key: Option<Encrypted<String>>,
    /// Its public half, to add to the repository.
    pub deploy_key_public: Option<String>,
    /// SHA-256 of the deployment trigger's token (`/api/deploy/…`).
    pub trigger_hash: Option<String>,
    /// A Compose service's file, written in the panel (`source = image`), or
    /// its path in the repository (`compose_file`, `source = git`).
    #[sea_orm(column_type = "Text", nullable)]
    pub compose: Option<String>,
    pub compose_file: Option<String>,
    /// Maintenance mode: its domains show the panel's maintenance page
    /// (this title and subtitle) instead of the service.
    pub maintenance: bool,
    pub maintenance_title: Option<String>,
    pub maintenance_subtitle: Option<String>,
    /// Notes about the service (its Overview), for the team.
    #[sea_orm(column_type = "Text", nullable)]
    pub notes: Option<String>,
    /// The GitHub webhook's secret (it checks the signature of a push),
    /// sealed (context `services.webhook_secret`).
    #[sea_orm(column_type = "Text", nullable)]
    pub webhook_secret: Option<Encrypted<String>>,
    pub command: Option<String>,
    pub replicas: i32,
    pub cpu_limit_millis: Option<i32>,
    pub mem_limit_mb: Option<i32>,
    pub cpu_reservation_millis: Option<i32>,
    pub mem_reservation_mb: Option<i32>,
    pub zero_downtime: bool,
    pub healthcheck_cmd: Option<String>,
    pub healthcheck_interval_s: Option<i32>,
    pub healthcheck_retries: Option<i32>,
    /// Environment variables, sealed with `APP_KEY` (context `services.env`).
    #[sea_orm(column_type = "Text", nullable)]
    pub env: Option<Encrypted<BTreeMap<String, String>>>,
    /// For a private registry (with `registry_password`); the registry is
    /// the image's.
    pub registry_username: Option<String>,
    /// Sealed with `APP_KEY` (context `services.registry_password`).
    #[sea_orm(column_type = "Text", nullable)]
    pub registry_password: Option<Encrypted<String>>,
    /// `running` or `stopped`: what the next deployment aims for.
    pub desired_state: String,
    /// `new`, `deploying`, `running`, `stopped`, `failed`.
    pub status: String,
    pub deployed_at: Option<DateTimeUtc>,
    pub deployed_spec_hash: Option<String>,
    /// The deployment holding the service (with `lock_expires_at`).
    pub current_deployment_id: Option<i64>,
    pub lock_expires_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Where `env` is sealed.
pub fn env_context() -> Context {
    Context::column("services", "env")
}

/// Where `webhook_secret` is sealed.
pub fn webhook_secret_context() -> Context {
    Context::column("services", "webhook_secret")
}

/// Where `deploy_key` is sealed.
pub fn deploy_key_context() -> Context {
    Context::column("services", "deploy_key")
}

/// Where `registry_password` is sealed.
pub fn registry_password_context() -> Context {
    Context::column("services", "registry_password")
}
