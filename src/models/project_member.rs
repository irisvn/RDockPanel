//! `project_members` table: a user's role in a project (`viewer`,
//! `developer`, `admin`: [`crate::services::access_service`]).

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, serde::Serialize)]
#[sea_orm(table_name = "project_members")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub project_id: i64,
    pub user_id: i64,
    pub role: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
