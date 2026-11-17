//! Importing existing containers (0.12.0): the imports (`container_imports`:
//! RustCMS has its own `imports` table, for CSV files), what each one moves
//! (items, volumes, files), the claims that keep two imports from taking the
//! same container or volume, `services.admin_only` (a service only
//! administrators change) and `service_mounts.link_target` (the folder a
//! `link` mount points to). Tables and columns are new: a release rolled
//! back keeps running on them.

use rustcms::async_trait;
use rustcms::migration::*;
use sea_orm_migration::sea_orm::{DatabaseBackend, Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mysql = manager.get_database_backend() == DatabaseBackend::MySql;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::AdminOnly)
                .boolean()
                .not_null()
                .default(false)
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            ServiceMounts::Table,
            &mut ColumnDef::new(ServiceMounts::LinkTarget)
                .string_len(1024)
                .null()
                .to_owned(),
        )
        .await?;

        // The plan is JSON up to 1 MiB: MySQL's TEXT holds 64 KiB.
        let mut plan = ColumnDef::new(ContainerImports::Plan);
        if mysql {
            plan.custom(Alias::new("MEDIUMTEXT"));
        } else {
            plan.text();
        }
        create_table(
            manager,
            Table::create()
                .table(ContainerImports::Table)
                .col(
                    ColumnDef::new(ContainerImports::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ContainerImports::ProjectId)
                        .big_integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImports::ProjectName)
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImports::SourceKind)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImports::SourceRef)
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImports::State)
                        .string_len(24)
                        .not_null(),
                )
                .col(plan.null().to_owned())
                .col(
                    ColumnDef::new(ContainerImports::Fingerprint)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImports::AutoUndo)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(ContainerImports::UndoRequested)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(ContainerImports::LeaseToken)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImports::LeaseExpiresAt)
                        .instant(manager)
                        .null(),
                )
                .col(ColumnDef::new(ContainerImports::Step).string_len(64).null())
                .col(ColumnDef::new(ContainerImports::Error).text().null())
                .col(
                    ColumnDef::new(ContainerImports::CreatedBy)
                        .big_integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImports::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImports::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImports::FinishedAt)
                        .instant(manager)
                        .null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ContainerImports::Table, ContainerImports::ProjectId)
                        .to(Projects::Table, Projects::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;

        create_table(
            manager,
            Table::create()
                .table(ContainerImportItems::Table)
                .col(
                    ColumnDef::new(ContainerImportItems::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::ImportId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::ServiceId)
                        .big_integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::ContainerId)
                        .string_len(80)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::ContainerName)
                        .string_len(255)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::ComposeService)
                        .string_len(255)
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::Level)
                        .integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::Choice)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::OriginalRestart)
                        .string_len(32)
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::WasRunning)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::StoppedAt)
                        .instant(manager)
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::PlannedHash)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::DeploymentId)
                        .big_integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportItems::State)
                        .string_len(24)
                        .not_null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ContainerImportItems::Table, ContainerImportItems::ImportId)
                        .to(ContainerImports::Table, ContainerImports::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ContainerImportItems::Table, ContainerImportItems::ServiceId)
                        .to(Services::Table, Services::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;

        // Kept even when the import or its service is deleted: the volume
        // holds data.
        create_table(
            manager,
            Table::create()
                .table(ContainerImportVolumes::Table)
                .col(
                    ColumnDef::new(ContainerImportVolumes::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ContainerImportVolumes::ImportId)
                        .big_integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportVolumes::OriginalName)
                        .string_len(1024)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportVolumes::Mode)
                        .string_len(8)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportVolumes::PanelVolume)
                        .string_len(255)
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportVolumes::Bytes)
                        .big_integer()
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportVolumes::CopiedAt)
                        .instant(manager)
                        .null(),
                )
                .col(
                    ColumnDef::new(ContainerImportVolumes::ReleasedAt)
                        .instant(manager)
                        .null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(
                            ContainerImportVolumes::Table,
                            ContainerImportVolumes::ImportId,
                        )
                        .to(ContainerImports::Table, ContainerImports::Id)
                        .on_delete(ForeignKeyAction::SetNull),
                )
                .to_owned(),
        )
        .await?;

        create_table(
            manager,
            Table::create()
                .table(ContainerImportClaims::Table)
                .col(
                    ColumnDef::new(ContainerImportClaims::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ContainerImportClaims::ImportId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportClaims::ContainerId)
                        .string_len(80)
                        .null()
                        .unique_key(),
                )
                .col(
                    ColumnDef::new(ContainerImportClaims::Volume)
                        .string_len(255)
                        .null()
                        .unique_key(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(
                            ContainerImportClaims::Table,
                            ContainerImportClaims::ImportId,
                        )
                        .to(ContainerImports::Table, ContainerImports::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

        create_table(
            manager,
            Table::create()
                .table(ContainerImportFiles::Table)
                .col(
                    ColumnDef::new(ContainerImportFiles::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ContainerImportFiles::ImportId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportFiles::Path)
                        .string_len(1024)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportFiles::BackupPath)
                        .string_len(1024)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportFiles::Sha256)
                        .string_len(64)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportFiles::Action)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ContainerImportFiles::RestoredAt)
                        .instant(manager)
                        .null(),
                )
                .foreign_key(
                    ForeignKey::create()
                        .from(ContainerImportFiles::Table, ContainerImportFiles::ImportId)
                        .to(ContainerImports::Table, ContainerImports::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Volumes copied by an import hold data, a folder in place is a
        // service's mount, `admin_only` keeps members out: refused before
        // anything changes. Each part is looked for first (a down that
        // stopped halfway on MySQL is run again).
        let count = |sql: &'static str| async move {
            manager
                .get_connection()
                .query_one_raw(Statement::from_string(
                    manager.get_database_backend(),
                    sql.to_owned(),
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("the rows could not be counted".to_owned()))?
                .try_get::<i64>("", "n")
        };
        if manager.has_table("container_imports").await?
            && count("SELECT COUNT(*) AS n FROM container_imports").await? > 0
        {
            return Err(DbErr::Custom(
                "imports exist: removing their tables would lose what they recorded".to_owned(),
            ));
        }
        if manager.has_column("service_mounts", "link_target").await?
            && count("SELECT COUNT(*) AS n FROM service_mounts WHERE kind = 'link'").await? > 0
        {
            return Err(DbErr::Custom(
                "services use folders in place: move them into volumes first".to_owned(),
            ));
        }
        if manager.has_column("services", "admin_only").await?
            && count("SELECT COUNT(*) AS n FROM services WHERE admin_only = TRUE").await? > 0
        {
            return Err(DbErr::Custom(
                "services are kept for administrators: removing the column would open them to members".to_owned(),
            ));
        }
        drop_table(manager, ContainerImportFiles::Table).await?;
        drop_table(manager, ContainerImportClaims::Table).await?;
        drop_table(manager, ContainerImportVolumes::Table).await?;
        drop_table(manager, ContainerImportItems::Table).await?;
        drop_table(manager, ContainerImports::Table).await?;
        drop_column(manager, ServiceMounts::Table, ServiceMounts::LinkTarget).await?;
        drop_column(manager, Services::Table, Services::AdminOnly).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Id,
    AdminOnly,
}

#[derive(DeriveIden)]
enum ServiceMounts {
    Table,
    LinkTarget,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum ContainerImports {
    Table,
    Id,
    ProjectId,
    ProjectName,
    SourceKind,
    SourceRef,
    State,
    Plan,
    Fingerprint,
    AutoUndo,
    UndoRequested,
    LeaseToken,
    LeaseExpiresAt,
    Step,
    Error,
    CreatedBy,
    CreatedAt,
    UpdatedAt,
    FinishedAt,
}

#[derive(DeriveIden)]
enum ContainerImportItems {
    Table,
    Id,
    ImportId,
    ServiceId,
    ContainerId,
    ContainerName,
    ComposeService,
    Level,
    Choice,
    OriginalRestart,
    WasRunning,
    StoppedAt,
    PlannedHash,
    DeploymentId,
    State,
}

#[derive(DeriveIden)]
enum ContainerImportVolumes {
    Table,
    Id,
    ImportId,
    OriginalName,
    Mode,
    PanelVolume,
    Bytes,
    CopiedAt,
    ReleasedAt,
}

#[derive(DeriveIden)]
enum ContainerImportClaims {
    Table,
    Id,
    ImportId,
    ContainerId,
    Volume,
}

#[derive(DeriveIden)]
enum ContainerImportFiles {
    Table,
    Id,
    ImportId,
    Path,
    BackupPath,
    Sha256,
    Action,
    RestoredAt,
}
