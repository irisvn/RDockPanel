//! `container_import_claims` table: what an unfinished import holds, so that two imports never take the same.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "container_import_claims")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub import_id: i64,
    /// Unique: a container belongs to one import at a time.
    pub container_id: Option<String>,
    /// Unique: so does a volume.
    pub volume: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
