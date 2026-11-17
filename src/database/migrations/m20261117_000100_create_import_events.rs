//! `container_import_events`: what an import did, step by step (stopped a
//! container, copied a volume, deployed a service…), for its page. Names
//! only, never a value. One row per `key` of an import: a step run again does
//! not repeat it.

use rustcms::async_trait;
use rustcms::migration::*;
use sea_orm_migration::sea_orm::Statement;

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
                .table(Events::Table)
                .col(
                    ColumnDef::new(Events::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Events::ImportId).big_integer().not_null())
                .col(ColumnDef::new(Events::EventKey).string_len(191).not_null())
                .col(ColumnDef::new(Events::Text).string_len(255).not_null())
                .col(ColumnDef::new(Events::Subject).string_len(255).null())
                .col(ColumnDef::new(Events::At).instant(manager).not_null())
                .foreign_key(
                    ForeignKey::create()
                        .from(Events::Table, Events::ImportId)
                        .to(Imports::Table, Imports::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Events::Table,
            Index::create()
                .name("container_import_events_key_unique")
                .table(Events::Table)
                .col(Events::ImportId)
                .col(Events::EventKey)
                .unique()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.has_table("container_import_events").await? {
            let rows = manager
                .get_connection()
                .query_one_raw(Statement::from_string(
                    manager.get_database_backend(),
                    "SELECT COUNT(*) AS n FROM container_import_events".to_owned(),
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("the events could not be counted".to_owned()))?
                .try_get::<i64>("", "n")?;
            if rows > 0 {
                return Err(DbErr::Custom(
                    "imports have recorded what they did: removing the table would lose it"
                        .to_owned(),
                ));
            }
        }
        drop_table(manager, Events::Table).await
    }
}

#[derive(DeriveIden)]
enum Events {
    #[sea_orm(iden = "container_import_events")]
    Table,
    Id,
    ImportId,
    #[sea_orm(iden = "event_key")]
    EventKey,
    Text,
    Subject,
    At,
}

#[derive(DeriveIden)]
enum Imports {
    #[sea_orm(iden = "container_imports")]
    Table,
    Id,
}
