//! Compose services: their Compose file (inline, or a file of their Git
//! repository), and which of their containers a domain leads to.

use rustcms::async_trait;
use rustcms::migration::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::Compose).text().null().to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::ComposeFile)
                .string_len(255)
                .null()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            ServiceDomains::Table,
            &mut ColumnDef::new(ServiceDomains::ComposeService)
                .string_len(64)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(
            manager,
            ServiceDomains::Table,
            ServiceDomains::ComposeService,
        )
        .await?;
        drop_column(manager, Services::Table, Services::ComposeFile).await?;
        drop_column(manager, Services::Table, Services::Compose).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Compose,
    ComposeFile,
}

#[derive(DeriveIden)]
enum ServiceDomains {
    Table,
    ComposeService,
}
