//! The `Networks` page (`/admin/p/networks`): networks: create and remove. Drawn by
//! `resources/admin/js/rdockpanel/networks.js`, which calls the routes of
//! `routes/admin.rs`.

use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct NetworksPage;

#[async_trait]
impl PanelPage for NetworksPage {
    const SLUG: &'static str = "networks";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Networks")
            .icon("network")
            .group("Docker")
            .sort(13)
            .permission("docker.view")
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/networks")
    }

    /// What the script may offer (the routes check again).
    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(
            json!({ "can": { "docker.manage": ctx.can("docker.manage"), "system.manage": ctx.can("system.manage") } }),
        )
    }
}
