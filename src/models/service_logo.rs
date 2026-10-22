//! `service_logos` table: a service's own logo on its maintenance page
//! (PNG, JPEG or WebP), one row per service.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "service_logos")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub service_id: i64,
    pub mime: String,
    #[sea_orm(column_type = "Blob")]
    pub data: Vec<u8>,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
