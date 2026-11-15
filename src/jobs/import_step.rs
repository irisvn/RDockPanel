//! The `import.step` job: one short step of an import (stop, copy, edit the
//! Compose file, deploy a level, undo), then the next is queued. The import
//! holds a lease: a job that finds it held does nothing, and the one that
//! lost the lease stops at its next check.

use std::time::Duration;

use rustcms::async_trait;
use rustcms::queue::{Job, JobCtx};
use serde::{Deserialize, Serialize};

use crate::services::import_run_service;

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportStep {
    pub import_id: i64,
}

#[async_trait]
impl Job for ImportStep {
    const KIND: &'static str = "import.step";
    const QUEUE: &'static str = "import";
    /// A step cut short is found again by the lease's expiry (`import-tick`),
    /// and each step is safe to run again.
    const AT_MOST_ONCE: bool = false;

    fn describe(&self) -> Option<String> {
        Some(format!("import #{}", self.import_id))
    }

    fn max_attempts() -> u32 {
        1
    }

    /// A copy or a deployment waits for Docker; the lease is renewed meanwhile.
    fn timeout() -> Duration {
        Duration::from_secs(30 * 60)
    }

    async fn handle(self, ctx: JobCtx) -> anyhow::Result<()> {
        import_run_service::step(&ctx.state, self.import_id).await?;
        Ok(())
    }
}
