//! `database_backups` table: a database service's backup configuration (a
//! database, a cron schedule in UTC, a storage provider and a path in it, how
//! many files to keep there). Its runs are `backup_runs`.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "database_backups")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub service_id: i64,
    pub storage_provider_id: i64,
    pub database_name: String,
    /// Whether the schedule runs it (it can always be run by hand).
    pub enabled: bool,
    /// Five-field cron, UTC.
    pub schedule: String,
    /// The files kept in `path` (older ones deleted after a backup).
    pub retention: i32,
    /// A folder of the provider (`production/orders`).
    pub path: String,
    /// The last slot of the schedule run (or skipped).
    pub last_slot_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
