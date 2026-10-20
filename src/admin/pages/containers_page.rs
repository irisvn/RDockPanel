//! The `Containers` page (`/admin/p/containers`): containers: state, logs, inspect and actions. Drawn by
//! `resources/admin/src/rdockpanel/docker/Containers.tsx` (React), which calls the routes of
//! `routes/admin.rs`.

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
            .permission("docker.view")
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/containers")
    }

    /// What the script may offer (the routes check again).
    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(
            json!({ "can": { "docker.manage": ctx.can("docker.manage"), "docker.logs": ctx.can("docker.logs"),
            "docker.exec": ctx.can("docker.exec"), "system.manage": ctx.can("system.manage") } }),
        )
    }
}
