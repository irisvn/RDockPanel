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

    /// `create_table` and the column helpers (`instant`, `instant_now`) give the
    /// same table on SQLite, PostgreSQL and MySQL, and skip what already exists
    /// (MySQL commits DDL statement by statement, so a failed run is retried).
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        create_table(
            manager,
            Table::create()
                .table(Actions::Table)
                .col(
                    ColumnDef::new(Actions::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Actions::Kind).string_len(64).not_null())
                .col(ColumnDef::new(Actions::Target).string_len(255).not_null())
                .col(ColumnDef::new(Actions::Status).string_len(16).not_null())
                .col(ColumnDef::new(Actions::UserId).big_integer().null())
                .col(ColumnDef::new(Actions::Log).text().null())
                .col(ColumnDef::new(Actions::Error).text().null())
                .col(ColumnDef::new(Actions::FinishedAt).instant(manager).null())
                .col(
                    ColumnDef::new(Actions::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Actions::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Actions::Table,
            Index::create()
                .name("actions_created_at_index")
                .table(Actions::Table)
                .col(Actions::CreatedAt)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, Actions::Table).await
    }
}

#[derive(DeriveIden)]
enum Actions {
    Table,
    Id,
    Kind,
    Target,
    Status,
    UserId,
    Log,
    Error,
    FinishedAt,
    CreatedAt,
    UpdatedAt,
}
