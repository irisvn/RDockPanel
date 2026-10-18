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
                .table(ServiceDomains::Table)
                .col(
                    ColumnDef::new(ServiceDomains::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ServiceDomains::ServiceId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceDomains::Host)
                        .string_len(253)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceDomains::Path)
                        .string_len(255)
                        .not_null()
                        .default("/"),
                )
                .col(ColumnDef::new(ServiceDomains::Port).integer().not_null())
                .col(
                    ColumnDef::new(ServiceDomains::Https)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(ServiceDomains::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceDomains::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ServiceDomains::Table, ServiceDomains::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            ServiceDomains::Table,
            Index::create()
                .name("service_domains_host_path_unique")
                .table(ServiceDomains::Table)
                .col(ServiceDomains::Host)
                .col(ServiceDomains::Path)
                .unique()
                .to_owned(),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, ServiceDomains::Table).await
    }
}

#[derive(DeriveIden)]
enum ServiceDomains {
    Table,
    Id,
    ServiceId,
    Host,
    Path,
    Port,
    Https,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
}
