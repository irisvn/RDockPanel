//! A domain's own Traefik middlewares (names defined in Traefik's files,
//! e.g. `ratelimit@file`), after the panel's.

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
            ServiceDomains::Table,
            &mut ColumnDef::new(ServiceDomains::Middlewares)
                .string_len(1000)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, ServiceDomains::Table, ServiceDomains::Middlewares).await
    }
}

#[derive(DeriveIden)]
enum ServiceDomains {
    Table,
    Middlewares,
}
