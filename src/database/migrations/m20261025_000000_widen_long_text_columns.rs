//! MySQL's TEXT holds 64 KiB, less than what the panel takes in these
//! columns (0.9.2): a Compose file (256 KiB), notes in characters of
//! several bytes, and a certificate chain. They become MEDIUMTEXT (16 MiB)
//! there; TEXT elsewhere holds them already. (The panel's settings became
//! MEDIUMTEXT with the maintenance page, 0.8.0.) Sealed values stay TEXT:
//! RustCMS seals 32 KiB at most (an envelope under 60 000 bytes).

use rustcms::async_trait;
use rustcms::migration::*;
use sea_orm_migration::sea_orm::{DatabaseBackend, Statement};

#[derive(DeriveMigrationName)]
pub struct Migration;

/// Each table's columns, and whether they take NULL.
const COLUMNS: &[(&str, &[(&str, bool)])] = &[
    ("services", &[("compose", true), ("notes", true)]),
    ("certificates", &[("chain", false)]),
];

async fn set_type(manager: &SchemaManager<'_>, kind: &str) -> Result<(), DbErr> {
    if manager.get_database_backend() != DatabaseBackend::MySql {
        return Ok(());
    }
    for (table, columns) in COLUMNS {
        let changes = columns
            .iter()
            .map(|(column, null)| {
                let null = if *null { "NULL" } else { "NOT NULL" };
                format!("MODIFY `{column}` {kind} {null}")
            })
            .collect::<Vec<_>>()
            .join(", ");
        manager
            .get_connection()
            .execute_unprepared(&format!("ALTER TABLE `{table}` {changes}"))
            .await?;
    }
    Ok(())
}

#[async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        set_type(manager, "MEDIUMTEXT").await
    }

    /// Back to TEXT: refused, before any change, while a value is longer.
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if manager.get_database_backend() == DatabaseBackend::MySql {
            for (table, columns) in COLUMNS {
                for (column, _) in *columns {
                    let longer = manager
                        .get_connection()
                        .query_one_raw(Statement::from_string(
                            DatabaseBackend::MySql,
                            format!(
                                "SELECT COUNT(*) AS n FROM `{table}` WHERE LENGTH(`{column}`) > 65535"
                            ),
                        ))
                        .await?
                        .map(|row| row.try_get::<i64>("", "n"))
                        .transpose()?
                        .unwrap_or(0);
                    if longer > 0 {
                        return Err(DbErr::Migration(format!(
                            "{longer} {table}.{column} value(s) are over 64 KiB: shorten them first"
                        )));
                    }
                }
            }
        }
        set_type(manager, "TEXT").await
    }
}
