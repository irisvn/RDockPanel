//! `service_mounts` table: a named volume (`rdp_<project>_<service>_<name>`)
//! or, for system managers, a host directory under an allowed root, or a
//! file with its content (a Swarm config; not for secrets: kept with each
//! deployment's configuration).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "service_mounts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub service_id: i64,
    /// `volume`, `bind` or `file`.
    pub kind: String,
    /// The volume's name (`volume`).
    pub name: Option<String>,
    /// The host directory (`bind`).
    pub source: Option<String>,
    pub target: String,
    /// The file's content (`file`).
    #[sea_orm(column_type = "Text", nullable)]
    pub content: Option<String>,
    pub read_only: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
