//! A service's rate limit (its Security tab): Traefik's `rateLimit` on its
//! domains, and the addresses exempt from it.

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
            &mut ColumnDef::new(Services::RateLimitAverage)
                .integer()
                .null()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::RateLimitPeriod)
                .string_len(8)
                .null()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::RateLimitBurst)
                .integer()
                .null()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::RateLimitExempt)
                .text()
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, Services::Table, Services::RateLimitExempt).await?;
        drop_column(manager, Services::Table, Services::RateLimitBurst).await?;
        drop_column(manager, Services::Table, Services::RateLimitPeriod).await?;
        drop_column(manager, Services::Table, Services::RateLimitAverage).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    RateLimitAverage,
    RateLimitPeriod,
    RateLimitBurst,
    RateLimitExempt,
}
