//! GitHub tokens (Settings → Git), sealed, each usable by the projects it
//! lists (a JSON array of their ids: a token reaches every repository its
//! account can, never a project's that was not given it), and the token a
//! service's repository is cloned with (none: its deploy key, or public).
//! A token in use cannot be deleted (checked by the panel).

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
                .table(GitProviders::Table)
                .col(
                    ColumnDef::new(GitProviders::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(GitProviders::Name).string_len(64).not_null())
                .col(ColumnDef::new(GitProviders::Kind).string_len(16).not_null())
                .col(ColumnDef::new(GitProviders::Token).text().not_null())
                .col(ColumnDef::new(GitProviders::Account).string_len(100).null())
                .col(ColumnDef::new(GitProviders::ProjectIds).text().not_null())
                .col(
                    ColumnDef::new(GitProviders::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(GitProviders::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            GitProviders::Table,
            Index::create()
                .name("git_providers_name_unique")
                .table(GitProviders::Table)
                .col(GitProviders::Name)
                .unique()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::GitProviderId)
                .big_integer()
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_column(manager, Services::Table, Services::GitProviderId).await?;
        drop_table(manager, GitProviders::Table).await
    }
}

#[derive(DeriveIden)]
enum GitProviders {
    Table,
    Id,
    Name,
    Kind,
    Token,
    Account,
    ProjectIds,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum Services {
    Table,
    GitProviderId,
}
