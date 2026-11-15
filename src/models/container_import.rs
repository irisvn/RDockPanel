//! `container_imports` table: one import of existing containers into a
//! project (docs/import.md). RustCMS has an `imports` table of its
//! own (CSV files), hence the name.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "container_imports")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub project_id: Option<i64>,
    pub project_name: String,
    /// `container`, `project` (a Compose project) or `move` (a service's data).
    pub source_kind: String,
    /// The container's id, or the Compose project's name.
    pub source_ref: String,
    pub state: String,
    /// The plan (JSON): names, ports, findings; never a secret.
    #[sea_orm(column_type = "Text", nullable)]
    pub plan: Option<String>,
    pub fingerprint: Option<String>,
    pub auto_undo: bool,
    pub undo_requested: bool,
    pub lease_token: Option<String>,
    pub lease_expires_at: Option<DateTimeUtc>,
    pub step: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub error: Option<String>,
    pub created_by: Option<i64>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub finished_at: Option<DateTimeUtc>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// The states in which the plan can be changed or the import deleted.
pub const EDITABLE: [&str; 3] = ["planned", "failed", "discover_failed"];
