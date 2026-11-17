//! Where a service runs (`services.placement`): empty, automatic (the
//! panel's node when only that machine has what it needs); `node:<id>`, that
//! Swarm node; `any`, any node. Set by the panel's administrators only.

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
            Services::Table,
            &mut ColumnDef::new(Services::Placement)
                .string_len(80)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A service placed by choice would move: refused before anything changes.
        if manager.has_column("services", "placement").await? {
            let set = manager
                .get_connection()
                .query_one_raw(Statement::from_string(
                    manager.get_database_backend(),
                    "SELECT COUNT(*) AS n FROM services WHERE placement IS NOT NULL AND placement <> ''"
                        .to_owned(),
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("the services could not be counted".to_owned()))?
                .try_get::<i64>("", "n")?;
            if set > 0 {
                return Err(DbErr::Custom(
                    "services are placed on a node of their choice: removing the column would move them"
                        .to_owned(),
                ));
            }
        }
        drop_column(manager, Services::Table, Services::Placement).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Placement,
}
