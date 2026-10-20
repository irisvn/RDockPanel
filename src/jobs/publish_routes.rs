//! The `deploy.publish` job: writes a service's Traefik routes again, as it
//! is deployed now with its routes' settings as saved: after a domain, a
//! redirect, a basic authentication user or maintenance changed (applied
//! without a deployment), and the service's Republish action.

use rustcms::async_trait;
use rustcms::queue::{Job, JobCtx};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};

use crate::models::deployment;
use crate::services::deployment_service;
use crate::services::traefik_service::Traefik;

#[derive(Debug, Serialize, Deserialize)]
pub struct PublishRoutes {
    pub service_id: i64,
}

#[async_trait]
impl Job for PublishRoutes {
    const KIND: &'static str = "deploy.publish";
    const QUEUE: &'static str = "default";

    async fn handle(self, ctx: JobCtx) -> anyhow::Result<()> {
        let Some(traefik) = ctx.state.service::<Traefik>() else {
            return Ok(());
        };
        let db = ctx.state.db();
        // Recorded on the latest deployment (what the service page shows).
        let Some(latest) = deployment::Entity::find()
            .filter(deployment::Column::ServiceId.eq(self.service_id))
            .order_by_desc(deployment::Column::Id)
            .one(db)
            .await?
        else {
            return Ok(());
        };
        let docker = ctx
            .state
            .service::<crate::services::docker_service::Docker>();
        // Traefik joins below, retried by this job (not queued again).
        if !deployment_service::publish(db, None, &traefik, self.service_id, latest.id).await? {
            anyhow::bail!("the routes of service {} were not written", self.service_id);
        }
        // Traefik off its network (Docker failing a moment): tried again.
        if let Some(docker) = docker
            && let Some(row) = crate::models::service::Entity::find_by_id(self.service_id)
                .one(db)
                .await?
        {
            deployment_service::traefik_reaches(db, &docker, &traefik, &row)
                .await
                .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }
}
