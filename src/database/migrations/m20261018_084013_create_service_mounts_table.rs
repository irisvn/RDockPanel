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
                .table(ServiceMounts::Table)
                .col(
                    ColumnDef::new(ServiceMounts::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ServiceMounts::ServiceId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceMounts::Kind)
                        .string_len(8)
                        .not_null()
                        .default("volume"),
                )
                .col(ColumnDef::new(ServiceMounts::Name).string_len(64).null())
                .col(ColumnDef::new(ServiceMounts::Source).string_len(512).null())
                .col(
                    ColumnDef::new(ServiceMounts::Target)
                        .string_len(512)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceMounts::ReadOnly)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(ServiceMounts::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ServiceMounts::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ServiceMounts::Table, ServiceMounts::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            ServiceMounts::Table,
            Index::create()
                .name("service_mounts_service_target_unique")
                .table(ServiceMounts::Table)
                .col(ServiceMounts::ServiceId)
                .col(ServiceMounts::Target)
                .unique()
                .to_owned(),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, ServiceMounts::Table).await
    }
}

#[derive(DeriveIden)]
enum ServiceMounts {
    Table,
    Id,
    ServiceId,
    Kind,
    Name,
    Source,
    Target,
    ReadOnly,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
}
