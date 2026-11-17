//! `container_import_events` table: what an import did, step by step (names only).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "container_import_events")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub import_id: i64,
    /// Unique for the import: a step run again does not repeat it.
    pub event_key: String,
    /// What happened, in English with `{name}` (translated by the page).
    pub text: String,
    /// What it happened to (a container, a volume, a service).
    pub subject: Option<String>,
    pub at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
