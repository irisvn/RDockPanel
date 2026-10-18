//! What a service is: an `app` (an image), or a database the panel
//! configures (`postgres`, `mysql`, `mariadb`, `mongo`, `redis`).

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
            Services::Table,
            ColumnDef::new(Services::Kind)
                .string_len(16)
                .not_null()
                .default("app"),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, Services::Table, Services::Kind).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Kind,
}
