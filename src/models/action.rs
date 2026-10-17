//! `actions` table: what the panel did to Docker, who asked, and how it
//! ended. Separate from the audit log, which records who changed what in the
//! panel's own data; user-initiated actions are written to both.

use sea_orm::entity::prelude::*;
use serde::Serialize;

pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const OK: &str = "ok";
pub const FAILED: &str = "failed";

/// `log` keeps at most this many bytes (the tail), below MySQL's 64 KiB TEXT.
pub const LOG_LIMIT: usize = 60 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize)]
#[sea_orm(table_name = "actions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    /// `container.restart`, `image.pull`, `system.prune`, …
    pub kind: String,
    /// The object acted on (container name, image reference, …).
    pub target: String,
    /// [`QUEUED`], [`RUNNING`], [`OK`] or [`FAILED`].
    pub status: String,
    pub user_id: Option<i64>,
    #[sea_orm(column_type = "Text", nullable)]
    pub log: Option<String>,
    #[sea_orm(column_type = "Text", nullable)]
    pub error: Option<String>,
    pub finished_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// The last [`LOG_LIMIT`] bytes of `log`, cut on a character boundary.
pub fn tail(log: &str) -> &str {
    if log.len() <= LOG_LIMIT {
        return log;
    }
    let mut start = log.len() - LOG_LIMIT;
    while !log.is_char_boundary(start) {
        start += 1;
    }
    &log[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_tail_of_long_logs() {
        assert_eq!(tail("short"), "short");
        let long = format!("{}é{}", "a".repeat(LOG_LIMIT), "b".repeat(10));
        let kept = tail(&long);
        assert!(kept.len() <= LOG_LIMIT && kept.ends_with("bbbbbbbbbb"));
    }
}
