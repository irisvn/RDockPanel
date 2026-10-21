//! `firewall_openings` table: a port (or range) the firewall opens, for its
//! sources only (or anywhere, chosen so); everything else is closed.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "firewall_openings")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub port_start: i32,
    pub port_end: i32,
    /// `tcp` or `udp`.
    pub protocol: String,
    /// `host` (this server's listeners) or `published` (a service's port).
    pub target: String,
    pub anywhere: bool,
    /// Its sources, canonical CIDRs separated by commas (both families).
    #[sea_orm(column_type = "Text")]
    pub sources: String,
    pub note: Option<String>,
    pub enabled: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
