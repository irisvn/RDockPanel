//! The WAF's own records, kept by the panel (CrowdSec's counters reset when
//! it restarts, its alerts live in a volume that may be reset):
//! - `waf_stats`: per hour, the requests the WAF checked and those it
//!   blocked (the panel's own test requests left out), and how many minutes
//!   were sampled (fewer than 60: the panel was stopped part of the hour);
//! - `waf_alerts`: CrowdSec's alerts, copied every minute (once each, by
//!   their uuid): when, who, the request (its query's secrets masked), the
//!   rule, blocked or recorded. Never a body.

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
        create_table(
            manager,
            Table::create()
                .table(Stats::Table)
                .col(
                    ColumnDef::new(Stats::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Stats::Hour).instant(manager).not_null())
                .col(
                    ColumnDef::new(Stats::Checked)
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(Stats::Blocked)
                        .big_integer()
                        .not_null()
                        .default(0),
                )
                .col(
                    ColumnDef::new(Stats::Samples)
                        .integer()
                        .not_null()
                        .default(0),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Stats::Table,
            Index::create()
                .name("waf_stats_hour_unique")
                .table(Stats::Table)
                .col(Stats::Hour)
                .unique()
                .to_owned(),
        )
        .await?;
        create_table(
            manager,
            Table::create()
                .table(Alerts::Table)
                .col(
                    ColumnDef::new(Alerts::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(ColumnDef::new(Alerts::Uuid).string_len(64).not_null())
                .col(ColumnDef::new(Alerts::At).instant(manager).not_null())
                .col(ColumnDef::new(Alerts::Ip).string_len(64).not_null())
                .col(ColumnDef::new(Alerts::Country).string_len(8).null())
                .col(ColumnDef::new(Alerts::Asn).string_len(191).null())
                .col(ColumnDef::new(Alerts::Host).string_len(255).null())
                .col(ColumnDef::new(Alerts::Path).string_len(512).null())
                .col(ColumnDef::new(Alerts::Rule).string_len(191).not_null())
                .col(ColumnDef::new(Alerts::RuleIds).string_len(255).null())
                .col(ColumnDef::new(Alerts::Blocked).boolean().not_null())
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Alerts::Table,
            Index::create()
                .name("waf_alerts_uuid_unique")
                .table(Alerts::Table)
                .col(Alerts::Uuid)
                .unique()
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            Alerts::Table,
            Index::create()
                .name("waf_alerts_at")
                .table(Alerts::Table)
                .col(Alerts::At)
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["waf_alerts", "waf_stats"] {
            if manager.has_table(table).await? {
                let rows = manager
                    .get_connection()
                    .query_one_raw(Statement::from_string(
                        manager.get_database_backend(),
                        format!("SELECT COUNT(*) AS n FROM {table}"),
                    ))
                    .await?
                    .ok_or_else(|| DbErr::Custom(format!("{table} could not be counted")))?
                    .try_get::<i64>("", "n")?;
                if rows > 0 {
                    return Err(DbErr::Custom(format!(
                        "the WAF has recorded {table}: removing the table would lose them"
                    )));
                }
            }
        }
        drop_table(manager, Alerts::Table).await?;
        drop_table(manager, Stats::Table).await
    }
}

#[derive(DeriveIden)]
enum Stats {
    #[sea_orm(iden = "waf_stats")]
    Table,
    Id,
    Hour,
    Checked,
    Blocked,
    Samples,
}

#[derive(DeriveIden)]
enum Alerts {
    #[sea_orm(iden = "waf_alerts")]
    Table,
    Id,
    Uuid,
    At,
    Ip,
    Country,
    Asn,
    Host,
    Path,
    Rule,
    RuleIds,
    Blocked,
}
