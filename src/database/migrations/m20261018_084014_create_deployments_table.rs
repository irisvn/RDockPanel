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
                .table(Deployments::Table)
                .col(
                    ColumnDef::new(Deployments::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(Deployments::ServiceId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Deployments::Trigger)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Deployments::Status)
                        .string_len(16)
                        .not_null(),
                )
                .col(ColumnDef::new(Deployments::Spec).json().not_null())
                .col(ColumnDef::new(Deployments::SpecHash).string_len(64).null())
                .col(
                    ColumnDef::new(Deployments::ImageDigest)
                        .string_len(255)
                        .null(),
                )
                .col(
                    ColumnDef::new(Deployments::Published)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(ColumnDef::new(Deployments::Log).text().null())
                .col(ColumnDef::new(Deployments::Error).text().null())
                .col(ColumnDef::new(Deployments::UserId).big_integer().null())
                .col(
                    ColumnDef::new(Deployments::StartedAt)
                        .instant(manager)
                        .null(),
                )
                .col(
                    ColumnDef::new(Deployments::FinishedAt)
                        .instant(manager)
                        .null(),
                )
                .col(
                    ColumnDef::new(Deployments::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Deployments::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(Deployments::Table, Deployments::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Deployments::Table,
            Index::create()
                .name("deployments_service_index")
                .table(Deployments::Table)
                .col(Deployments::ServiceId)
                .to_owned(),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, Deployments::Table).await
    }
}

#[derive(DeriveIden)]
enum Deployments {
    Table,
    Id,
    ServiceId,
    Trigger,
    Status,
    Spec,
    SpecHash,
    ImageDigest,
    Published,
    Log,
    Error,
    UserId,
    StartedAt,
    FinishedAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
}
