//! The `Firewall` page (`/admin/p/firewall`): the server's listeners and
//! its firewall (default deny; SSH, 80 and 443 open; the ports opened for
//! their sources). Administrators' (`system.manage`). A React page of the
//! dashboard's bundle: `resources/admin/src/rdockpanel/firewall/`.

use crate::policies::permissions;
use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct FirewallPage;

#[async_trait]
impl PanelPage for FirewallPage {
    const SLUG: &'static str = "firewall";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("Firewall")
            .icon("shield")
            .sort(2)
            .permission_any([permissions::SYSTEM_MANAGE])
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/firewall")
    }

    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(json!({ "user": ctx.principal.id }))
    }
}
