//! A service's WAF setting (`services.waf`): empty, it follows the panel's
//! (Settings → Security); `on` or `off` for this service. Only the panel's
//! administrators lower it (`waf_service::may_change`).

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
            &mut ColumnDef::new(Services::Waf)
                .string_len(8)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A service with a WAF setting of its own would lose it: refused first.
        if manager.has_column("services", "waf").await? {
            let set = manager
                .get_connection()
                .query_one_raw(Statement::from_string(
                    manager.get_database_backend(),
                    "SELECT COUNT(*) AS n FROM services WHERE waf IS NOT NULL".to_owned(),
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("the services could not be counted".to_owned()))?
                .try_get::<i64>("", "n")?;
            if set > 0 {
                return Err(DbErr::Custom(
                    "services have a WAF setting of their own: removing the column would lose it"
                        .to_owned(),
                ));
            }
        }
        drop_column(manager, Services::Table, Services::Waf).await
    }
}

#[derive(DeriveIden)]
enum Services {
    Table,
    Waf,
}
