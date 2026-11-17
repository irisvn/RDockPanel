//! `container_import_items.tunnel_id`: the tunnel an import made of a
//! cloudflared (its row in `tunnels`, not its name: a name can be made
//! again), so that an undo removes that one and no other.

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
            ContainerImportItems::Table,
            &mut ColumnDef::new(ContainerImportItems::TunnelId)
                .big_integer()
                .null()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager
            .has_column("container_import_items", "tunnel_id")
            .await?
        {
            return Ok(());
        }
        let made = manager
            .get_connection()
            .query_one_raw(Statement::from_string(
                manager.get_database_backend(),
                "SELECT COUNT(*) AS n FROM container_import_items WHERE tunnel_id IS NOT NULL"
                    .to_owned(),
            ))
            .await?
            .ok_or_else(|| DbErr::Custom("the import items could not be counted".to_owned()))?
            .try_get::<i64>("", "n")?;
        if made > 0 {
            return Err(DbErr::Custom(
                "imports made tunnels: removing the column would forget which ones an undo removes"
                    .to_owned(),
            ));
        }
        drop_column(
            manager,
            ContainerImportItems::Table,
            ContainerImportItems::TunnelId,
        )
        .await
    }
}

#[derive(DeriveIden)]
enum ContainerImportItems {
    Table,
    TunnelId,
}
