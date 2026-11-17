//! `container_import_volumes.cleaning_at`: when the admin started to delete the
//! old folder of a move. From then the move cannot be undone (the folder it
//! would go back to is being emptied), and a clean that failed is run again.

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
        add_column(
            manager,
            Volumes::Table,
            &mut ColumnDef::new(Volumes::CleaningAt)
                .instant(manager)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_column("container_import_volumes", "cleaning_at")
            .await?
        {
            return Ok(());
        }
        // A folder being deleted: without the mark its move could be undone
        // onto a folder half gone. Refused before anything changes.
        let open = manager
            .get_connection()
            .query_one_raw(Statement::from_string(
                manager.get_database_backend(),
                "SELECT COUNT(*) AS n FROM container_import_volumes \
                 WHERE cleaning_at IS NOT NULL AND released_at IS NULL"
                    .to_owned(),
            ))
            .await?
            .ok_or_else(|| DbErr::Custom("the import volumes could not be counted".to_owned()))?
            .try_get::<i64>("", "n")?;
        if open > 0 {
            return Err(DbErr::Custom(
                "an old folder is being cleaned: finish the clean first".to_owned(),
            ));
        }
        drop_column(manager, Volumes::Table, Volumes::CleaningAt).await
    }
}

#[derive(DeriveIden)]
enum Volumes {
    #[sea_orm(iden = "container_import_volumes")]
    Table,
    CleaningAt,
}
