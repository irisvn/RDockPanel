//! The events a notification channel hears, by name (a JSON array). None:
//! as before it existed (its deployment, backup and disk choices, and the
//! panel's own alerts).

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
            NotificationChannels::Table,
            &mut ColumnDef::new(NotificationChannels::Events)
                .text()
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(
            manager,
            NotificationChannels::Table,
            NotificationChannels::Events,
        )
        .await
    }
}

#[derive(DeriveIden)]
enum NotificationChannels {
    Table,
    Events,
}
