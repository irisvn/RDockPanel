//! `services.user`: the user a service's container runs as (`root`, `1000:1000`),
//! when it is not its image's. Only an import sets it (a container that ran
//! as another user than its image's): the form cannot.

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
            &mut ColumnDef::new(Services::RunUser)
                .string_len(64)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Refused before anything changes: a service that runs as root would
        // run as its image's user, and members could change it.
        if manager.has_column("services", "run_user").await? {
            let held = manager
                .get_connection()
                .query_one_raw(Statement::from_string(
                    manager.get_database_backend(),
                    "SELECT COUNT(*) AS n FROM services WHERE run_user IS NOT NULL".to_owned(),
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("the rows could not be counted".to_owned()))?
                .try_get::<i64>("", "n")?;
            if held > 0 {
                return Err(DbErr::Custom(
                    "services run as a user of their own: removing the column would change who they run as".to_owned(),
                ));
            }
        }
        drop_column(manager, Services::Table, Services::RunUser).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    #[sea_orm(iden = "run_user")]
    RunUser,
}
