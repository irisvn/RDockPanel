//! `container_import_volumes` table: a volume (or folder) an import copies or links: kept until it is released.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "container_import_volumes")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// Kept (NULL) when the import or its service is deleted: the volume holds data.
    pub import_id: Option<i64>,
    pub original_name: String,
    /// `copy` or `link`.
    pub mode: String,
    pub panel_volume: Option<String>,
    pub bytes: Option<i64>,
    pub copied_at: Option<DateTimeUtc>,
    pub released_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
