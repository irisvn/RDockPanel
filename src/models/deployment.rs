//! `deployments` table: one run of a service's deployment (deploy, start,
//! stop, restart, destroy, rollback), with the configuration it applied
//! (`spec`: names of its environment variables, never their values) for
//! history and rollback, and its environment sealed apart (`env`).

use std::collections::BTreeMap;

use rustcms::crypto::{Context, Encrypted};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "deployments")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub service_id: i64,
    /// `deploy`, `start`, `stop`, `restart`, `destroy`, `rollback`.
    pub trigger: String,
    /// `queued`, `running`, `succeeded`, `failed`, `rolled_back`, `interrupted`.
    pub status: String,
    pub spec: Json,
    pub spec_hash: Option<String>,
    pub image_digest: Option<String>,
    /// Whether its routes reached Traefik.
    pub published: bool,
    #[sea_orm(column_type = "Text", nullable)]
    pub log: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub error: Option<String>,
    pub user_id: Option<i64>,
    /// The environment it applies, sealed with `APP_KEY` (context
    /// `deployments.env`): a rollback applies it again.
    #[sea_orm(column_type = "Text", nullable)]
    pub env: Option<Encrypted<BTreeMap<String, String>>>,
    /// Its file mounts' contents by path, sealed (context `deployments.files`).
    #[sea_orm(column_type = "Text", nullable)]
    pub files: Option<Encrypted<BTreeMap<String, String>>>,
    pub started_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Where `env` is sealed.
pub fn env_context() -> Context {
    Context::column("deployments", "env")
}

/// Where `files` is sealed.
pub fn files_context() -> Context {
    Context::column("deployments", "files")
}
