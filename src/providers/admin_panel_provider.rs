//! The admin panel at `/admin` (like a Filament panel provider): its
//! resources (`rustcms make:resource`), pages (`make:settings`) and labels.

use rustcms::prelude::*;

pub fn provider() -> AdminProvider {
    AdminProvider::new()
        .brand("RDockPanel")
        // Labels in Vietnamese (`rustcms lang:missing vi` lists what is missing).
        .translations("vi", include_str!("../../lang/vi.toml"))
        .resource::<crate::admin::resources::project_resource::ProjectResource>()
        .resource::<crate::admin::resources::service_resource::ServiceResource>()
        .resource::<crate::admin::resources::service_domain_resource::ServiceDomainResource>()
        .resource::<crate::admin::resources::service_port_resource::ServicePortResource>()
        .resource::<crate::admin::resources::service_mount_resource::ServiceMountResource>()
        .resource::<crate::admin::resources::deployment_resource::DeploymentResource>()
        .resource::<crate::admin::resources::database_backup_resource::DatabaseBackupResource>()
        .resource::<crate::admin::resources::storage_provider_resource::StorageProviderResource>()
        .resource::<crate::admin::resources::notification_channel_resource::NotificationChannelResource>()
        .resource::<crate::admin::resources::service_redirect_resource::ServiceRedirectResource>()
        // rustcms:resources
        // The apps interface first (React, host API v2).
        .page::<crate::admin::pages::dashboard_page::DashboardPage>()
        .page::<crate::admin::pages::actions_page::ActionsPage>()
        .page::<crate::admin::pages::settings_page::SettingsPage>()
        .page::<crate::admin::pages::server_page::ServerPage>()
        .page::<crate::admin::pages::containers_page::ContainersPage>()
        .page::<crate::admin::pages::images_page::ImagesPage>()
        .page::<crate::admin::pages::volumes_page::VolumesPage>()
        .page::<crate::admin::pages::networks_page::NetworksPage>()
        // rustcms:pages
        .slot::<crate::admin::slots::service_live_slot::ServiceLiveSlot>()
        // The terminal's xterm.js, its CSP shim first (resources/admin/js/rdockpanel/vendor/README.md).
        .extension_script(
            "xterm-csp-shim",
            include_str!("../../resources/admin/js/rdockpanel/vendor/csp-shim.js"),
        )
        .extension_script(
            "xterm",
            include_str!("../../resources/admin/js/rdockpanel/vendor/xterm.js"),
        )
        .extension_script(
            "xterm-fit",
            include_str!("../../resources/admin/js/rdockpanel/vendor/addon-fit.js"),
        )
        .extension_style(
            "xterm",
            include_str!("../../resources/admin/css/rdockpanel/xterm.css"),
        )
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
        .extension_script(
            "service-live",
            include_str!("../../resources/admin/js/rdockpanel/service-live.js"),
        )
        .extension_style(
            "panel",
            include_str!("../../resources/admin/css/rdockpanel/panel.css"),
        )
        .extension_script_v2(
            "dashboard",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/resources/admin/js/rdockpanel/dashboard.js"
            )),
            rustcms::admin::Requires {
                react: "19.3",
                react_query: "5.104",
                ui: 1,
            },
        )
        .extension_style(
            "dashboard",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/resources/admin/css/rdockpanel/dashboard.css"
            )),
        )
    // rustcms:extensions
}
