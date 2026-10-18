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
                .table(Projects::Table)
                .col(
                    ColumnDef::new(Projects::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Projects::Name).string_len(40).not_null())
                .col(ColumnDef::new(Projects::Description).text().null())
                .col(
                    ColumnDef::new(Projects::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Projects::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Projects::Table,
            Index::create()
                .name("projects_name_unique")
                .table(Projects::Table)
                .col(Projects::Name)
                .unique()
                .to_owned(),
        )
        .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, Projects::Table).await
    }
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    Id,
    Name,
    Description,
    CreatedAt,
    UpdatedAt,
}
