//! `service_domains` table: a host (and path) routed by Traefik to a port of
//! the service. One service per host and path.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "service_domains")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub service_id: i64,
    pub host: String,
    pub path: String,
    pub port: i32,
    pub https: bool,
    /// A Compose service's container (its service in the Compose file).
    pub compose_service: Option<String>,
    /// Traefik middlewares of its own, comma-separated (`ratelimit@file`).
    pub middlewares: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
