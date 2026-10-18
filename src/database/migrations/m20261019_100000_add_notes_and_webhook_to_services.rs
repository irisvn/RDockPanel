//! A service's notes (its Overview), and the secret of its GitHub webhook
//! (auto deploy on push, signed; sealed: it signs, so it is kept in clear).

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
            &mut ColumnDef::new(Services::Notes).text().null().to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::WebhookSecret)
                .text()
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, Services::Table, Services::WebhookSecret).await?;
        drop_column(manager, Services::Table, Services::Notes).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Notes,
    WebhookSecret,
}
