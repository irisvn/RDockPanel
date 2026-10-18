//! `deployments` table: one run of a service's deployment (deploy, start,
//! stop, restart, destroy, rollback), with the configuration it applied
//! (`spec`, never a secret value) for history and rollback.

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
    pub started_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
