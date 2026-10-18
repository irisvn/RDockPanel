//! `service_mounts` table: a named volume (`rdp_<project>_<service>_<name>`)
//! or, for system managers, a host directory under an allowed root.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "service_mounts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub service_id: i64,
    /// `volume` or `bind`.
    pub kind: String,
    /// The volume's name (`volume`).
    pub name: Option<String>,
    /// The host directory (`bind`).
    pub source: Option<String>,
    pub target: String,
    pub read_only: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
