//! The Firewall page's policy and its changes: the ports opened (each for
//! its sources), the blocked addresses, and every change made on the host
//! (a transaction: applied, then kept or reverted; docs/firewall.md).

use rustcms::async_trait;
use rustcms::migration::*;

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
                .table(FirewallOpenings::Table)
                .col(
                    ColumnDef::new(FirewallOpenings::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(FirewallOpenings::PortStart)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallOpenings::PortEnd)
                        .integer()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallOpenings::Protocol)
                        .string_len(8)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallOpenings::Target)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallOpenings::Anywhere)
                        .boolean()
                        .not_null()
                        .default(false),
                )
                .col(ColumnDef::new(FirewallOpenings::Sources).text().not_null())
                .col(
                    ColumnDef::new(FirewallOpenings::Note)
                        .string_len(200)
                        .null(),
                )
                .col(
                    ColumnDef::new(FirewallOpenings::Enabled)
                        .boolean()
                        .not_null()
                        .default(true),
                )
                .col(
                    ColumnDef::new(FirewallOpenings::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallOpenings::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_table(
            manager,
            Table::create()
                .table(FirewallBlocked::Table)
                .col(
                    ColumnDef::new(FirewallBlocked::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(FirewallBlocked::Cidr)
                        .string_len(64)
                        .not_null(),
                )
                .col(ColumnDef::new(FirewallBlocked::Note).string_len(200).null())
                .col(
                    ColumnDef::new(FirewallBlocked::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            FirewallBlocked::Table,
            Index::create()
                .name("firewall_blocked_cidr_unique")
                .table(FirewallBlocked::Table)
                .col(FirewallBlocked::Cidr)
                .unique()
                .to_owned(),
        )
        .await?;
        create_table(
            manager,
            Table::create()
                .table(FirewallTransactions::Table)
                .col(
                    ColumnDef::new(FirewallTransactions::Id)
                        .big_integer()
                        .not_null()
                        .auto_increment()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(FirewallTransactions::Txid)
                        .string_len(32)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallTransactions::Kind)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallTransactions::State)
                        .string_len(16)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallTransactions::Ruleset)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallTransactions::Summary)
                        .text()
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallTransactions::Hash)
                        .string_len(64)
                        .null(),
                )
                .col(
                    ColumnDef::new(FirewallTransactions::Deadline)
                        .instant(manager)
                        .null(),
                )
                .col(
                    ColumnDef::new(FirewallTransactions::CreatedBy)
                        .big_integer()
                        .null(),
                )
                .col(ColumnDef::new(FirewallTransactions::Result).text().null())
                .col(
                    ColumnDef::new(FirewallTransactions::CreatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .col(
                    ColumnDef::new(FirewallTransactions::UpdatedAt)
                        .instant_now(manager)
                        .not_null(),
                )
                .to_owned(),
        )
        .await?;
        create_index(
            manager,
            FirewallTransactions::Table,
            Index::create()
                .name("firewall_transactions_txid_unique")
                .table(FirewallTransactions::Table)
                .col(FirewallTransactions::Txid)
                .unique()
                .to_owned(),
        )
        .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        drop_table(manager, FirewallTransactions::Table).await?;
        drop_table(manager, FirewallBlocked::Table).await?;
        drop_table(manager, FirewallOpenings::Table).await
    }
}

#[derive(DeriveIden)]
enum FirewallOpenings {
    Table,
    Id,
    PortStart,
    PortEnd,
    Protocol,
    Target,
    Anywhere,
    Sources,
    Note,
    Enabled,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum FirewallBlocked {
    Table,
    Id,
    Cidr,
    Note,
    CreatedAt,
}

#[derive(DeriveIden)]
enum FirewallTransactions {
    Table,
    Id,
    Txid,
    Kind,
    State,
    Ruleset,
    Summary,
    Hash,
    Deadline,
    CreatedBy,
    Result,
    CreatedAt,
    UpdatedAt,
}
