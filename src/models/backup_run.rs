//! `backup_runs` table: one backup or restore of a database service, with its
//! log (what the Backups tab and the Actions feed show).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "backup_runs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub service_id: i64,
    pub backup_id: Option<i64>,
    pub storage_provider_id: Option<i64>,
    /// `backup` or `restore`.
    pub kind: String,
    /// `manual` or `schedule`.
    pub trigger: String,
    /// `queued`, `running`, `succeeded`, `failed`.
    pub status: String,
    pub database_name: String,
    /// The file in the provider (written, or restored from), named when the
    /// run is asked for.
    pub file: Option<String>,
    pub size: Option<i64>,
    /// A backup: the files kept, as configured when it was asked for.
    pub retention: Option<i32>,
    /// While it runs: the container its command runs in, and the mark its
    /// processes carry (killed when the run is found dead).
    pub container: Option<String>,
    pub mark: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub log: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub error: Option<String>,
    pub user_id: Option<i64>,
    pub started_at: Option<DateTimeUtc>,
    pub finished_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
