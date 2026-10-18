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
                .table(Services::Table)
                .col(
                    ColumnDef::new(Services::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Services::ProjectId).big_integer().not_null())
                .col(ColumnDef::new(Services::Name).string_len(40).not_null())
                .col(ColumnDef::new(Services::Image).string_len(255).not_null())
                .col(ColumnDef::new(Services::Command).string_len(1000).null())
                .col(
                    ColumnDef::new(Services::Replicas)
                        .integer()
                        .not_null()
                        .default(1),
                )
                .col(ColumnDef::new(Services::CpuLimitMillis).integer().null())
                .col(ColumnDef::new(Services::MemLimitMb).integer().null())
                .col(
                    ColumnDef::new(Services::CpuReservationMillis)
                        .integer()
                        .null(),
                )
                .col(ColumnDef::new(Services::MemReservationMb).integer().null())
                .col(
                    ColumnDef::new(Services::ZeroDowntime)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(Services::HealthcheckCmd)
                        .string_len(500)
                        .null(),
                )
                .col(
                    ColumnDef::new(Services::HealthcheckIntervalS)
                        .integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(Services::HealthcheckRetries)
                        .integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(Services::DesiredState)
                        .string_len(16)
                        .not_null()
                        .default("running"),
                )
                .col(
                    ColumnDef::new(Services::Status)
                        .string_len(16)
                        .not_null()
                        .default("new"),
                )
                .col(ColumnDef::new(Services::DeployedAt).instant(manager).null())
                .col(
                    ColumnDef::new(Services::DeployedSpecHash)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(Services::CurrentDeploymentId)
                        .big_integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(Services::LockExpiresAt)
                        .instant(manager)
                        .null(),
                )
                .col(
                    ColumnDef::new(Services::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Services::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(Services::Table, Services::ProjectId)
                        .to(Projects::Table, Projects::Id)
                        .on_delete(ForeignKeyAction::Restrict),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Services::Table,
            Index::create()
                .name("services_project_name_unique")
                .table(Services::Table)
                .col(Services::ProjectId)
                .col(Services::Name)
                .unique()
                .to_owned(),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, Services::Table).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
    ProjectId,
    Name,
    Image,
    Command,
    Replicas,
    CpuLimitMillis,
    MemLimitMb,
    CpuReservationMillis,
    MemReservationMb,
    ZeroDowntime,
    HealthcheckCmd,
    HealthcheckIntervalS,
    HealthcheckRetries,
    DesiredState,
    Status,
    DeployedAt,
    DeployedSpecHash,
    CurrentDeploymentId,
    LockExpiresAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    Id,
}
