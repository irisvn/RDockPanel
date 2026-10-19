//! A project's members (users of the admin panel) and their role in it:
//! `viewer`, `developer` or `admin` (`access_service`). One row per user
//! and project; removed with the project (checked by the panel: it is
//! deleted only once empty, its members then).

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
                .table(ProjectMembers::Table)
                .col(
                    ColumnDef::new(ProjectMembers::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(ProjectMembers::ProjectId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ProjectMembers::UserId)
                        .big_integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ProjectMembers::Role)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ProjectMembers::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(ProjectMembers::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            ProjectMembers::Table,
            Index::create()
                .name("project_members_project_user_unique")
                .table(ProjectMembers::Table)
                .col(ProjectMembers::ProjectId)
                .col(ProjectMembers::UserId)
                .unique()
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            ProjectMembers::Table,
            Index::create()
                .name("project_members_user")
                .table(ProjectMembers::Table)
                .col(ProjectMembers::UserId)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, ProjectMembers::Table).await
    }
}

#[derive(DeriveIden)]
enum ProjectMembers {
    Table,
    Id,
    ProjectId,
    UserId,
    Role,
    CreatedAt,
    UpdatedAt,
}
