//! The admin panel at `/admin` (like a Filament panel provider): its
//! resources (`rustcms make:resource`), pages (`make:settings`) and labels.

use rustcms::prelude::*;

pub fn provider() -> AdminProvider {
    AdminProvider::new()
        .brand("RDockPanel")
        // Labels in Vietnamese (`rustcms lang:missing vi` lists what is missing).
        .translations("vi", include_str!("../../lang/vi.toml"))
        // rustcms:resources
        .page::<crate::admin::pages::server_page::ServerPage>()
        .page::<crate::admin::pages::containers_page::ContainersPage>()
        .page::<crate::admin::pages::images_page::ImagesPage>()
        .page::<crate::admin::pages::volumes_page::VolumesPage>()
        .page::<crate::admin::pages::networks_page::NetworksPage>()
        // rustcms:pages
        // Shared helpers first: the page scripts use `window.rdockPanel`.
        .extension_script(
            "panel",
            include_str!("../../resources/admin/js/rdockpanel/panel.js"),
        )
        .extension_script(
            "server",
            include_str!("../../resources/admin/js/rdockpanel/server.js"),
        )
        .extension_script(
            "containers",
            include_str!("../../resources/admin/js/rdockpanel/containers.js"),
        )
        .extension_script(
            "images",
            include_str!("../../resources/admin/js/rdockpanel/images.js"),
        )
        .extension_script(
            "volumes",
            include_str!("../../resources/admin/js/rdockpanel/volumes.js"),
        )
        .extension_script(
            "networks",
            include_str!("../../resources/admin/js/rdockpanel/networks.js"),
        )
        .extension_style(
            "panel",
            include_str!("../../resources/admin/css/rdockpanel/panel.css"),
        )
    // rustcms:extensions
}
