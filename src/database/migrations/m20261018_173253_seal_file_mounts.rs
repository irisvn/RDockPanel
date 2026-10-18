//! A file mount's content is sealed with `APP_KEY` like an environment
//! (templates put passwords in files): the column holds an envelope from now
//! on (the plain contents of before are dropped: no release had them), and
//! each deployment keeps the files it applied, sealed (`deployments.files`).

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
        drop_column(manager, ServiceMounts::Table, ServiceMounts::Content).await?;
        add_column(
            manager,
            ServiceMounts::Table,
            ColumnDef::new(ServiceMounts::SealedContent).text().null(),
        )
        .await?;
        add_column(
            manager,
            Deployments::Table,
            ColumnDef::new(Deployments::Files).text().null(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, Deployments::Table, Deployments::Files).await?;
        drop_column(manager, ServiceMounts::Table, ServiceMounts::SealedContent).await?;
        add_column(
            manager,
            ServiceMounts::Table,
            ColumnDef::new(ServiceMounts::Content).text().null(),
        )
        .await
    }
}

#[derive(DeriveIden)]
enum ServiceMounts {
    Table,
    Content,
    SealedContent,
}

#[derive(DeriveIden)]
enum Deployments {
    Table,
    Files,
}
