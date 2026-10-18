//! The `backup.run` job: a database backup or restore asked for by hand or
//! by its schedule (`backup_service::request`, in the same transaction).
//! One attempt: a restore is never replayed blindly; a run cut short is
//! marked failed at boot (`backup_service::recover_interrupted`).

use std::time::Duration;

use rustcms::async_trait;
use rustcms::queue::{Job, JobCtx};
use serde::{Deserialize, Serialize};

use crate::services::backup_service;

#[derive(Debug, Serialize, Deserialize)]
pub struct RunBackup {
    pub run_id: i64,
}

#[async_trait]
impl Job for RunBackup {
    const KIND: &'static str = "backup.run";
    const QUEUE: &'static str = "backup";

    fn max_attempts() -> u32 {
        1
    }

    fn timeout() -> Duration {
        Duration::from_secs(3 * 60 * 60)
    }

    async fn handle(self, ctx: JobCtx) -> anyhow::Result<()> {
        backup_service::run(&ctx.state, self.run_id).await?;
        Ok(())
    }
}
