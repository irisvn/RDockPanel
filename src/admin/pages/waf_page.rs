//! The `WAF` page (`/admin/p/waf`): the web application firewall in front
//! of the services' domains (CrowdSec's AppSec): its state, statistics,
//! alerts, rules and exceptions, blocklist, Cloudflare's ranges and
//! settings. Administrators' (`system.manage`). A React page of the
//! dashboard's bundle: `resources/admin/src/rdockpanel/waf/`.

use crate::policies::permissions;
use rustcms::admin::{Ctx, PageView, PanelPage, PanelPageMeta};
use rustcms::{AppResult, async_trait};
use serde_json::{Value, json};

pub struct WafPage;

#[async_trait]
impl PanelPage for WafPage {
    const SLUG: &'static str = "waf";

    fn meta() -> PanelPageMeta {
        PanelPageMeta::new("WAF")
            .icon("activity")
            .sort(3)
            .permission_any([permissions::SYSTEM_MANAGE])
    }

    fn view() -> PageView {
        PageView::Script("rdockpanel/waf")
    }

    async fn data(ctx: &Ctx) -> AppResult<Value> {
        Ok(json!({ "user": ctx.principal.id }))
    }
}
