//! The `docker.image_pull` job: pulls an image and keeps Docker's progress
//! as the log of its `actions` row. Dispatched by
//! `docker_controller::pull_image` in the transaction that creates the row.

use std::time::Duration;

use futures_util::StreamExt;
use rustcms::async_trait;
use rustcms::queue::{Job, JobCtx};
use serde::{Deserialize, Serialize};

use crate::models::action;
use crate::services::action_service;
use crate::services::docker_service::Docker;

/// The `actions` row and the reference to pull (`name[:tag]`).
#[derive(Debug, Serialize, Deserialize)]
pub struct PullImage {
    pub action_id: i64,
    pub image: String,
}

#[async_trait]
impl Job for PullImage {
    const KIND: &'static str = "docker.image_pull";
    const QUEUE: &'static str = "docker";

    /// A failed pull is reported on its row; pulling again is the user's call.
    fn max_attempts() -> u32 {
        1
    }

    fn timeout() -> Duration {
        Duration::from_secs(30 * 60)
    }

    async fn handle(self, ctx: JobCtx) -> anyhow::Result<()> {
        let db = ctx.state.db();
        let Some(row) = action_service::running(db, self.action_id).await? else {
            return Ok(());
        };
        let mut log = String::new();
        // Whatever happens, the row ends (a crash or timeout is caught by
        // `action_service::reap_stale`).
        let error = match ctx.state.service::<Docker>() {
            Some(docker) => pull(&docker, &self.image, &mut log).await.err(),
            None => Some("Docker is not bound".to_owned()),
        };
        action_service::finish(db, row, error, Some(&log)).await?;
        Ok(())
    }
}

/// Pulls `image`, keeping the tail of Docker's progress in `log` (never more
/// than twice what the row keeps).
async fn pull(docker: &Docker, image: &str, log: &mut String) -> Result<(), String> {
    let mut progress = docker.pull_image(image);
    while let Some(line) = progress.next().await {
        let line = line.map_err(|err| err.to_string())?;
        log.push_str(action::tail(&line));
        log.push('\n');
        if log.len() > 2 * action::LOG_LIMIT {
            *log = action::tail(log).to_owned();
        }
    }
    Ok(())
}
