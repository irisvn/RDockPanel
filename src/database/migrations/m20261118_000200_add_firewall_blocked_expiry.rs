//! A blocked address's end (`firewall_blocked.expires_at`): empty, it stays
//! blocked until removed; else the panel removes it then and applies the
//! firewall again (`ip_block_service::expire`).

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
            FirewallBlocked::Table,
            &mut ColumnDef::new(FirewallBlocked::ExpiresAt)
                .instant(manager)
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // A block with an end would become permanent: refused first.
        if manager.has_column("firewall_blocked", "expires_at").await? {
            let set = manager
                .get_connection()
                .query_one_raw(Statement::from_string(
                    manager.get_database_backend(),
                    "SELECT COUNT(*) AS n FROM firewall_blocked WHERE expires_at IS NOT NULL"
                        .to_owned(),
                ))
                .await?
                .ok_or_else(|| DbErr::Custom("the blocks could not be counted".to_owned()))?
                .try_get::<i64>("", "n")?;
            if set > 0 {
                return Err(DbErr::Custom(
                    "blocked addresses have an end: removing the column would make them permanent"
                        .to_owned(),
                ));
            }
        }
        drop_column(manager, FirewallBlocked::Table, FirewallBlocked::ExpiresAt).await
    }
}

#[derive(DeriveIden)]
enum FirewallBlocked {
    Table,
    ExpiresAt,
}
