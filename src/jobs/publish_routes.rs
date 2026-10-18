//! The `deploy.publish` job: writes a service's Traefik routes again, as it
//! is deployed now (the service's Republish action, after a failed write).

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
        if !deployment_service::publish(db, &traefik, self.service_id, latest.id).await? {
            anyhow::bail!("the routes of service {} were not written", self.service_id);
        }
        Ok(())
    }
}
