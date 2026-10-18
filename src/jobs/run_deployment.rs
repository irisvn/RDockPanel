//! The `deploy.run` job: runs a deployment asked for by a service's action
//! (`deployment_service::request`, in the same transaction). One attempt: a
//! deployment is never replayed blindly; a run cut short is found at boot
//! (`deployment_service::recover_interrupted`).

use std::time::Duration;

use rustcms::async_trait;
use rustcms::queue::{Job, JobCtx};
use serde::{Deserialize, Serialize};

use crate::services::deployment_service::{self, Timings};

#[derive(Debug, Serialize, Deserialize)]
pub struct RunDeployment {
    pub deployment_id: i64,
}

#[async_trait]
impl Job for RunDeployment {
    const KIND: &'static str = "deploy.run";
    const QUEUE: &'static str = "deploy";

    fn max_attempts() -> u32 {
        1
    }

    /// More than a run waits for Swarm (`Timings::timeout`). Its lease is
    /// renewed meanwhile; a run aborted at this timeout lets it expire.
    fn timeout() -> Duration {
        Duration::from_secs(15 * 60)
    }

    async fn handle(self, ctx: JobCtx) -> anyhow::Result<()> {
        deployment_service::run(&ctx.state, self.deployment_id, Timings::default()).await?;
        Ok(())
    }
}
