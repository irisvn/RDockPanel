//! `git_providers` table: GitHub tokens (Settings → Git), sealed with
//! `APP_KEY` (context `git_providers.token`), each usable by the projects
//! it lists.

use rustcms::crypto::{Context, Encrypted};
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "git_providers")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub name: String,
    /// `github`.
    pub kind: String,
    #[sea_orm(column_type = "Text")]
    pub token: Encrypted<String>,
    /// The account the token signs in as (its last check).
    pub account: Option<String>,
    /// The projects that may use it: a JSON array of their ids.
    #[sea_orm(column_type = "Text")]
    pub project_ids: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// Where `token` is sealed.
pub fn token_context() -> Context {
    Context::column("git_providers", "token")
}

impl Model {
    /// The projects that may use it.
    pub fn projects(&self) -> Vec<i64> {
        serde_json::from_str(&self.project_ids).unwrap_or_default()
    }

    /// Whether `project` may use it.
    pub fn allows(&self, project: i64) -> bool {
        self.projects().contains(&project)
    }
}
