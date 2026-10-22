//! The `Containers` page (`/admin/p/containers`): containers: state, logs, inspect and actions. Drawn by
//! `resources/admin/src/rdockpanel/docker/Containers.tsx` (React), which calls the routes of
//! `routes/admin.rs`.

use crate::policies::permissions;
use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct ContainersPage;

#[async_trait]
impl PanelPage for ContainersPage {
    const SLUG: &'static str = "containers";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Containers")
            .icon("container")
            .group("Docker")
            .sort(10)
            .permission(permissions::DOCKER_VIEW)
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/containers")
    }

    /// What the script may offer (the routes check again).
    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(
            json!({ "can": { permissions::DOCKER_MANAGE: ctx.can(permissions::DOCKER_MANAGE), permissions::DOCKER_LOGS: ctx.can(permissions::DOCKER_LOGS),
            permissions::DOCKER_EXEC: ctx.can(permissions::DOCKER_EXEC), permissions::SYSTEM_MANAGE: ctx.can(permissions::SYSTEM_MANAGE) } }),
        )
    }
}
