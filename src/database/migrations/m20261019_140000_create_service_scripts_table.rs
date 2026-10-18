//! A service's scripts (as EasyPanel's): named shell commands, run in one of
//! its running containers from its Scripts tab.

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
                .table(ServiceScripts::Table)
                .col(
                    ColumnDef::new(ServiceScripts::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ServiceScripts::ServiceId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceScripts::Name)
                        .string_len(64)
                        .not_null(),
                )
                .col(ColumnDef::new(ServiceScripts::Command).text().not_null())
                .col(
                    ColumnDef::new(ServiceScripts::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceScripts::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ServiceScripts::Table, ServiceScripts::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            ServiceScripts::Table,
            Index::create()
                .name("service_scripts_service_index")
                .table(ServiceScripts::Table)
                .col(ServiceScripts::ServiceId)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, ServiceScripts::Table).await
    }
}

#[derive(DeriveIden)]
enum ServiceScripts {
    Table,
    Id,
    ServiceId,
    Name,
    Command,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
}
