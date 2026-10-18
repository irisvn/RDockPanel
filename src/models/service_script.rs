//! `service_scripts` table: a service's scripts (its Scripts tab): a name
//! and a shell command, run in one of its running containers.

use sea_orm::entity::prelude::*;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
#[sea_orm(table_name = "service_scripts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub service_id: i64,
    pub name: String,
    #[sea_orm(column_type = "Text")]
    pub command: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
