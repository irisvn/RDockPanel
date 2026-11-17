//! A service's health check, as an imported container had it: its timeout and
//! start period (`healthcheck_timeout_s`, `healthcheck_start_period_s`, the
//! deployment's defaults when empty), and `healthcheck_off`: the check is
//! turned off (Docker's `NONE`), so the image's own does not run either.

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
            &mut ColumnDef::new(Services::HealthcheckTimeoutS)
                .integer()
                .null()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::HealthcheckStartPeriodS)
                .integer()
                .null()
                .to_owned(),
        )
        .await?;
        add_column(
            manager,
            Services::Table,
            &mut ColumnDef::new(Services::HealthcheckOff)
                .boolean()
                .not_null()
                .default(false)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A service whose check is off or timed its own way would change how
        // it is checked: refused before anything changes.
        if manager.has_column("services", "healthcheck_off").await? {
            let set = manager
                .get_connection()
                .query_one_raw(Statement::from_string(
                    manager.get_database_backend(),
                    "SELECT COUNT(*) AS n FROM services WHERE healthcheck_off = TRUE \
                     OR healthcheck_timeout_s IS NOT NULL OR healthcheck_start_period_s IS NOT NULL"
                        .to_owned(),
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("the services could not be counted".to_owned()))?
                .try_get::<i64>("", "n")?;
            if set > 0 {
                return Err(DbErr::Custom(
                    "services have health check settings of their own: removing the columns would change them"
                        .to_owned(),
                ));
            }
        }
        drop_column(manager, Services::Table, Services::HealthcheckOff).await?;
        drop_column(manager, Services::Table, Services::HealthcheckStartPeriodS).await?;
        drop_column(manager, Services::Table, Services::HealthcheckTimeoutS).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    HealthcheckTimeoutS,
    HealthcheckStartPeriodS,
    HealthcheckOff,
}
