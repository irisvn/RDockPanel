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
    pub image: String,
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

/// Where `registry_password` is sealed.
pub fn registry_password_context() -> Context {
    Context::column("services", "registry_password")
}
