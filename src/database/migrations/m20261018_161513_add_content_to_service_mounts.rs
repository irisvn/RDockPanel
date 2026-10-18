//! A file mount's content (`kind = 'file'`): a Swarm config at deploy time.

use rustcms::async_trait;
use rustcms::migration::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    /// Schema change and migration record commit together, also on SQLite
    /// (whose default is no transaction), so a failed run can be retried.
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_column(
            manager,
            ServiceMounts::Table,
            ColumnDef::new(ServiceMounts::Content).text().null(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, ServiceMounts::Table, ServiceMounts::Content).await
    }
}

#[derive(DeriveIden)]
enum ServiceMounts {
    Table,
    Content,
}
