//! `service_ports` table: a port published on every node (`ingress`) or on
//! the node running the task (`host`). One service per port and protocol.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "service_ports")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub service_id: i64,
    pub published: i32,
    pub target: i32,
    /// `tcp` or `udp`.
    pub protocol: String,
    /// `ingress` or `host`.
    pub mode: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
