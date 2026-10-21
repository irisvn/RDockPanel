//! `firewall_blocked` table: an address or network denied every port but
//! SSH (canonical CIDR).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "firewall_blocked")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub cidr: String,
    pub note: Option<String>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
