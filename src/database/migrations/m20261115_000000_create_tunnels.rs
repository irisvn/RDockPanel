//! `tunnels`: the panel's Cloudflare Tunnels (one cloudflared each), their
//! token sealed (MEDIUMTEXT on MySQL, like the other sealed columns). The
//! one of `panel_settings` (`tunnel.*`) becomes the row `default` at boot
//! (`tunnel_service::load`: sealing again needs the APP_KEY, which a
//! migration has not); the settings stay, for a release rolled back.

use rustcms::async_trait;
use rustcms::migration::*;
use sea_orm_migration::sea_orm::{DatabaseBackend, Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait]
impl MigrationTrait for Migration {
    fn use_transaction(&self) -> Option<bool> {
        Some(true)
    }

    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let mut token = ColumnDef::new(Tunnels::Token);
        if manager.get_database_backend() == DatabaseBackend::MySql {
            token.custom(Alias::new("MEDIUMTEXT"));
        } else {
            token.text();
        }
        create_table(
            manager,
            Table::create()
                .table(Tunnels::Table)
                .col(
                    ColumnDef::new(Tunnels::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Tunnels::Name).string_len(40).not_null())
                .col(token.not_null())
                .col(
                    ColumnDef::new(Tunnels::Enabled)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(
                    ColumnDef::new(Tunnels::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(Tunnels::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Tunnels::Table,
            Index::create()
                .name("tunnels_name_unique")
                .table(Tunnels::Table)
                .col(Tunnels::Name)
                .unique()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if !manager.has_table("tunnels").await? {
            return Ok(());
        }
        // `default` is kept in the settings too; another tunnel's token only here.
        let others = manager
            .get_connection()
            .query_one_raw(Statement::from_string(
                manager.get_database_backend(),
                "SELECT COUNT(*) AS n FROM tunnels WHERE name <> 'default'".to_owned(),
            ))
            .await?
            .and_then(|row| row.try_get::<i64>("", "n").ok())
            .unwrap_or(0);
        if others > 0 {
            return Err(DbErr::Custom(
                "tunnels other than the default exist: removing the table would lose their tokens"
                    .to_owned(),
            ));
        }
        drop_table(manager, Tunnels::Table).await
    }
}

#[derive(DeriveIden)]
enum Tunnels {
    Table,
    Id,
    Name,
    Token,
    Enabled,
    CreatedAt,
    UpdatedAt,
}
