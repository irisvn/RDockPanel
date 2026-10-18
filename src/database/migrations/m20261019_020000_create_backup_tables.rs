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
                .table(StorageProviders::Table)
                .col(
                    ColumnDef::new(StorageProviders::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(StorageProviders::Name)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(StorageProviders::Kind)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(StorageProviders::LocalPath)
                        .string_len(512)
                        .null(),
                )
                .col(
                    ColumnDef::new(StorageProviders::Endpoint)
                        .string_len(512)
                        .null(),
                )
                .col(
                    ColumnDef::new(StorageProviders::Region)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(StorageProviders::Bucket)
                        .string_len(255)
                        .null(),
                )
                .col(
                    ColumnDef::new(StorageProviders::AccessKey)
                        .string_len(255)
                        .null(),
                )
                .col(ColumnDef::new(StorageProviders::SecretKey).text().null())
                .col(
                    ColumnDef::new(StorageProviders::PathStyle)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(StorageProviders::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(StorageProviders::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            StorageProviders::Table,
            Index::create()
                .name("storage_providers_name_unique")
                .table(StorageProviders::Table)
                .col(StorageProviders::Name)
                .unique()
                .to_owned(),
        )
        .await?;

        create_table(
            manager,
            Table::create()
                .table(DatabaseBackups::Table)
                .col(
                    ColumnDef::new(DatabaseBackups::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::ServiceId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::StorageProviderId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::DatabaseName)
                        .string_len(128)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::Enabled)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::Schedule)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::Retention)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::Path)
                        .string_len(512)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::LastSlotAt)
                        .instant(manager)
                        .null(),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(DatabaseBackups::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(DatabaseBackups::Table, DatabaseBackups::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(DatabaseBackups::Table, DatabaseBackups::StorageProviderId)
                        .to(StorageProviders::Table, StorageProviders::Id)
                        .on_delete(ForeignKeyAction::Restrict),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            DatabaseBackups::Table,
            Index::create()
                .name("database_backups_service_index")
                .table(DatabaseBackups::Table)
                .col(DatabaseBackups::ServiceId)
                .to_owned(),
        )
        .await?;

        create_table(
            manager,
            Table::create()
                .table(BackupRuns::Table)
                .col(
                    ColumnDef::new(BackupRuns::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(BackupRuns::ServiceId)
                        .big_integer()
                        .not_null(),
                )
                .col(ColumnDef::new(BackupRuns::BackupId).big_integer().null())
                .col(
                    ColumnDef::new(BackupRuns::StorageProviderId)
                        .big_integer()
                        .null(),
                )
                .col(ColumnDef::new(BackupRuns::Kind).string_len(16).not_null())
                .col(
                    ColumnDef::new(BackupRuns::Trigger)
                        .string_len(16)
                        .not_null(),
                )
                .col(ColumnDef::new(BackupRuns::Status).string_len(16).not_null())
                .col(
                    ColumnDef::new(BackupRuns::DatabaseName)
                        .string_len(128)
                        .not_null(),
                )
                .col(ColumnDef::new(BackupRuns::File).string_len(1024).null())
                .col(ColumnDef::new(BackupRuns::Size).big_integer().null())
                .col(ColumnDef::new(BackupRuns::Retention).integer().null())
                .col(ColumnDef::new(BackupRuns::Log).text().null())
                .col(ColumnDef::new(BackupRuns::Error).text().null())
                .col(ColumnDef::new(BackupRuns::UserId).big_integer().null())
                .col(
                    ColumnDef::new(BackupRuns::StartedAt)
                        .instant(manager)
                        .null(),
                )
                .col(
                    ColumnDef::new(BackupRuns::FinishedAt)
                        .instant(manager)
                        .null(),
                )
                .col(
                    ColumnDef::new(BackupRuns::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(BackupRuns::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(BackupRuns::Table, BackupRuns::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(BackupRuns::Table, BackupRuns::BackupId)
                        .to(DatabaseBackups::Table, DatabaseBackups::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(BackupRuns::Table, BackupRuns::StorageProviderId)
                        .to(StorageProviders::Table, StorageProviders::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            BackupRuns::Table,
            Index::create()
                .name("backup_runs_service_index")
                .table(BackupRuns::Table)
                .col(BackupRuns::ServiceId)
                .to_owned(),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, BackupRuns::Table).await?;
        drop_table(manager, DatabaseBackups::Table).await?;
        drop_table(manager, StorageProviders::Table).await
    }
}

#[derive(DeriveIden)]
enum StorageProviders {
    Table,
    Id,
    Name,
    Kind,
    LocalPath,
    Endpoint,
    Region,
    Bucket,
    AccessKey,
    SecretKey,
    PathStyle,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum DatabaseBackups {
    Table,
    Id,
    ServiceId,
    StorageProviderId,
    DatabaseName,
    Enabled,
    Schedule,
    Retention,
    Path,
    LastSlotAt,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum BackupRuns {
    Table,
    Id,
    ServiceId,
    BackupId,
    StorageProviderId,
    Kind,
    Trigger,
    Status,
    DatabaseName,
    File,
    Size,
    Retention,
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
