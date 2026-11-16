//! `container_import_items` table: one container of an import: the service made for it, and what the original was.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "container_import_items")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub import_id: i64,
    pub service_id: Option<i64>,
    pub container_id: String,
    pub container_name: String,
    pub compose_service: Option<String>,
    pub level: i32,
    /// `bring`, `leave` or `blocked`.
    pub choice: String,
    /// The restart policy the original had (`always`, `no`, …).
    pub original_restart: Option<String>,
    pub was_running: bool,
    pub stopped_at: Option<DateTimeUtc>,
    pub planned_hash: Option<String>,
    pub deployment_id: Option<i64>,
    /// `pending`, `stopped`, `copied`, `deployed`, `restored`.
    pub state: String,
    /// The tunnel made of this cloudflared (`tunnels.id`): removed by an undo.
    pub tunnel_id: Option<i64>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
