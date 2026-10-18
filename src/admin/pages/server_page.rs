//! The `Server` page (`/admin/p/server`): the host: CPU, memory, network and disk, live. Drawn by
//! `resources/admin/js/rdockpanel/server.js`, which calls the routes of
//! `routes/admin.rs`.

use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct ServerPage;

#[async_trait]
impl PanelPage for ServerPage {
    const SLUG: &'static str = "server";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Server")
            .icon("box")
            .group("Server")
            .permission("server.view")
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/server")
    }

    /// What the script may offer (the routes check again).
    async fn data(_ctx: &Ctx) -> AppResult<Value> {
        Ok(json!({}))
    }
}
