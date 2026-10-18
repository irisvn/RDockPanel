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
                .table(ServicePorts::Table)
                .col(
                    ColumnDef::new(ServicePorts::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ServicePorts::ServiceId)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(ServicePorts::Published).integer().not_null())
                .col(ColumnDef::new(ServicePorts::Target).integer().not_null())
                .col(
                    ColumnDef::new(ServicePorts::Protocol)
                        .string_len(3)
                        .not_null()
                        .default("tcp"),
                )
                .col(
                    ColumnDef::new(ServicePorts::Mode)
                        .string_len(8)
                        .not_null()
                        .default("ingress"),
                )
                .col(
                    ColumnDef::new(ServicePorts::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServicePorts::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ServicePorts::Table, ServicePorts::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            ServicePorts::Table,
            Index::create()
                .name("service_ports_published_protocol_unique")
                .table(ServicePorts::Table)
                .col(ServicePorts::Published)
                .col(ServicePorts::Protocol)
                .unique()
                .to_owned(),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, ServicePorts::Table).await
    }
}

#[derive(DeriveIden)]
enum ServicePorts {
    Table,
    Id,
    ServiceId,
    Published,
    Target,
    Protocol,
    Mode,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
}
