//! The panel's own settings changed in its interface (Traefik's custom
//! middlewares, …): one value per key.

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
        create_table(
            manager,
            Table::create()
                .table(PanelSettings::Table)
                .col(
                    ColumnDef::new(PanelSettings::Key)
                        .string_len(64)
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(PanelSettings::Value).text().not_null())
                .col(
                    ColumnDef::new(PanelSettings::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, PanelSettings::Table).await
    }
}

#[derive(DeriveIden)]
enum PanelSettings {
    Table,
    Key,
    Value,
    UpdatedAt,
}
