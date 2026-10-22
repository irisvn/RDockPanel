//! The maintenance page, designed again (0.8.0): when the service is back
//! (and the offset it was written in), a contact, the page (the default
//! one or its own HTML), the addresses that still reach the service, and
//! a logo of its own. The panel's settings hold a page shared by every
//! service (MySQL's TEXT holds 64 KiB: MEDIUMTEXT there).

use rustcms::async_trait;
use rustcms::migration::*;
use sea_orm_migration::sea_orm::DatabaseBackend;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mysql = manager.get_database_backend() == DatabaseBackend::MySql;
        for column in [
            ColumnDef::new(Services::MaintenanceEta)
                .instant(manager)
                .null()
                .to_owned(),
            ColumnDef::new(Services::MaintenanceEtaOffset)
                .integer()
                .null()
                .to_owned(),
            ColumnDef::new(Services::MaintenanceContact)
                .string_len(255)
                .null()
                .to_owned(),
            ColumnDef::new(Services::MaintenancePage)
                .string_len(16)
                .not_null()
                .default("default")
                .to_owned(),
            {
                let mut html = ColumnDef::new(Services::MaintenanceHtml);
                if mysql {
                    html.custom(Alias::new("MEDIUMTEXT"));
                } else {
                    html.text();
                }
                html.null().to_owned()
            },
            ColumnDef::new(Services::MaintenanceBypass)
                .text()
                .null()
                .to_owned(),
        ] {
            add_column(manager, Services::Table, &mut column.clone()).await?;
        }
        let mut data = ColumnDef::new(ServiceLogos::Data);
        if mysql {
            data.custom(Alias::new("MEDIUMBLOB"));
        } else {
            data.blob();
        }
        create_table(
            manager,
            Table::create()
                .table(ServiceLogos::Table)
                .col(
                    ColumnDef::new(ServiceLogos::ServiceId)
                        .big_integer()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(ServiceLogos::Mime).string_len(32).not_null())
                .col(data.not_null())
                .col(
                    ColumnDef::new(ServiceLogos::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ServiceLogos::Table, ServiceLogos::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        if mysql {
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE panel_settings MODIFY `value` MEDIUMTEXT NOT NULL")
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, ServiceLogos::Table).await?;
        for column in [
            Services::MaintenanceBypass,
            Services::MaintenanceHtml,
            Services::MaintenancePage,
            Services::MaintenanceContact,
            Services::MaintenanceEtaOffset,
            Services::MaintenanceEta,
        ] {
            drop_column(manager, Services::Table, column).await?;
        }
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
    MaintenanceEta,
    MaintenanceEtaOffset,
    MaintenanceContact,
    MaintenancePage,
    MaintenanceHtml,
    MaintenanceBypass,
}

#[derive(DeriveIden)]
enum ServiceLogos {
    Table,
    ServiceId,
    Mime,
    Data,
    UpdatedAt,
}
