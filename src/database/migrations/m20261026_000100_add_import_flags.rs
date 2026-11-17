//! `container_imports.flags`: what an import has become, apart from where it
//! is (`ingress`: traffic could have reached a new service; `back`: a move is
//! on its way back). Kept apart from `step` (where to go on) so that a
//! failure that waits for the admin forgets neither.

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
            ContainerImports::Table,
            &mut ColumnDef::new(ContainerImports::Flags)
                .string_len(64)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Refused before anything changes: an import's flags say where an undo
        // goes (`back`) and whether it may undo by itself (`ingress`).
        if manager.has_column("container_imports", "flags").await? {
            let held = manager
                .get_connection()
                .query_one_raw(Statement::from_string(
                    manager.get_database_backend(),
                    "SELECT COUNT(*) AS n FROM container_imports WHERE flags IS NOT NULL"
                        .to_owned(),
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("the rows could not be counted".to_owned()))?
                .try_get::<i64>("", "n")?;
            if held > 0 {
                return Err(DbErr::Custom(
                    "imports have flags: removing the column would forget how to undo them"
                        .to_owned(),
                ));
            }
        }
        drop_column(manager, ContainerImports::Table, ContainerImports::Flags).await
    }
}

#[derive(DeriveIden)]
enum ContainerImports {
    Table,
    Flags,
}
