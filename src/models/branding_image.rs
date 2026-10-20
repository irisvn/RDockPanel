//! `branding_images` table: Settings → Branding's logo, dark logo and
//! favicon, one row per kind (checked as the panel serves them).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "branding_images")]
pub struct Model {
    /// `logo`, `logo-dark` or `favicon`.
    #[sea_orm(primary_key, auto_increment = false)]
    pub kind: String,
    pub mime: String,
    #[sea_orm(column_type = "Blob")]
    pub data: Vec<u8>,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
