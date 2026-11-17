//! `container_import_files` table: a Compose file an import changed, and its backup.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "container_import_files")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub import_id: i64,
    pub path: String,
    pub backup_path: String,
    pub sha256: String,
    /// `rename` or `profile`.
    pub action: String,
    pub restored_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
