//! A service's secrets, sealed with `APP_KEY` (RustCMS `Encrypted<T>`, text
//! columns): its environment variables and its registry password; and the
//! environment each deployment applied (sealed too), for rollback.

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

    /// Column by column, each skipped when present (MySQL commits each DDL
    /// at once: a run cut short is taken up where it stopped).
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_column(
            manager,
            Services::Table,
            ColumnDef::new(Services::Env).text().null(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            ColumnDef::new(Services::RegistryUsername)
                .string_len(255)
                .null(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            ColumnDef::new(Services::RegistryPassword).text().null(),
        )
        .await?;
        add_column(
            manager,
            Deployments::Table,
            ColumnDef::new(Deployments::Env).text().null(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, Deployments::Table, Deployments::Env).await?;
        drop_column(manager, Services::Table, Services::RegistryPassword).await?;
        drop_column(manager, Services::Table, Services::RegistryUsername).await?;
        drop_column(manager, Services::Table, Services::Env).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Env,
    RegistryUsername,
    RegistryPassword,
}

#[derive(DeriveIden)]
enum Deployments {
    Table,
    Env,
}
