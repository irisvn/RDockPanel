//! The `import.discover` job: reads the containers an import is about (Docker,
//! read only) and writes its plan. Asked by `POST /imports` and a rescan, in
//! the transaction that audits `imports.discover`.

use std::time::Duration;

use rustcms::async_trait;
use rustcms::queue::{Job, JobCtx};
use serde::{Deserialize, Serialize};

use crate::services::import_service;

#[derive(Debug, Serialize, Deserialize)]
pub struct ImportDiscover {
    pub import_id: i64,
}

#[async_trait]
impl Job for ImportDiscover {
    const KIND: &'static str = "import.discover";
    const QUEUE: &'static str = "import";
    /// Reading changes nothing: a replay is harmless.
    const AT_MOST_ONCE: bool = false;

    fn describe(&self) -> Option<String> {
        Some(format!("import #{}", self.import_id))
    }

    fn max_attempts() -> u32 {
        1
    }

    fn timeout() -> Duration {
        Duration::from_secs(120)
    }

    async fn handle(self, ctx: JobCtx) -> anyhow::Result<()> {
        import_service::discover(&ctx.state, self.import_id).await?;
        Ok(())
    }
}
