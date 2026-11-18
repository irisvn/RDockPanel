//! `waf_stats` table: per hour, the requests the WAF checked and blocked
//! (the panel's own test requests left out), and the minutes sampled.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "waf_stats")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// The hour's start (UTC).
    pub hour: DateTimeUtc,
    pub checked: i64,
    pub blocked: i64,
    /// Minutes sampled: fewer than 60, the panel was stopped part of it.
    pub samples: i32,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
