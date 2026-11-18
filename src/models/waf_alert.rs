//! `waf_alerts` table: CrowdSec's alerts copied by the panel (once each,
//! by their uuid), kept 30 days. The request's query has its secrets
//! masked; never a body.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "waf_alerts")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub uuid: String,
    pub at: DateTimeUtc,
    pub ip: String,
    pub country: Option<String>,
    pub asn: Option<String>,
    pub host: Option<String>,
    pub path: Option<String>,
    /// The rule matched (Block: the scenario; Detect: the rule's name).
    pub rule: String,
    /// Every rule id that matched, as CrowdSec lists them.
    pub rule_ids: Option<String>,
    pub blocked: bool,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
