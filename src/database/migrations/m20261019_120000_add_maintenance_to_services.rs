//! A service's maintenance mode (as EasyPanel's): its domains show a page of
//! the panel (a title, a subtitle) instead of the service.

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
            &mut ColumnDef::new(Services::Maintenance)
                .boolean()
                .not_null()
                .default(false)
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::MaintenanceTitle)
                .string_len(200)
                .null()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::MaintenanceSubtitle)
                .string_len(1000)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, Services::Table, Services::MaintenanceSubtitle).await?;
        drop_column(manager, Services::Table, Services::MaintenanceTitle).await?;
        drop_column(manager, Services::Table, Services::Maintenance).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Maintenance,
    MaintenanceTitle,
    MaintenanceSubtitle,
}
