//! `firewall_transactions` table: each change of the host's firewall, as
//! the panel recorded it before asking the host (`intent`), then
//! `pending` (applied, its revert armed), and at last `kept`, `reverted` or
//! `failed`; the host's journal is the truth it follows (docs/firewall.md).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "firewall_transactions")]
pub struct Model {
    /// Also the revision (`rev-<id>.nft` on the host).
    #[sea_orm(primary_key)]
    pub id: i64,
    pub txid: String,
    /// `apply` (`disable` and `reconcile` are recorded as actions).
    pub kind: String,
    pub state: String,
    /// The table it loads.
    #[sea_orm(column_type = "Text")]
    pub ruleset: String,
    /// What it opens and blocks (JSON), as shown.
    #[sea_orm(column_type = "Text")]
    pub summary: String,
    /// The loaded table's canonical hash (as the kernel listed it).
    pub hash: Option<String>,
    /// Kept before, or the host reverts it.
    pub deadline: Option<DateTimeUtc>,
    pub created_by: Option<i64>,
    #[sea_orm(column_type = "Text", nullable)]
    pub result: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Not over yet (at most one at a time).
pub fn open(state: &str) -> bool {
    matches!(state, "intent" | "pending" | "unknown")
}
