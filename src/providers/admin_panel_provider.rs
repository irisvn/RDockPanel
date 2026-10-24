//! The admin panel at `/admin` (like a Filament panel provider): its
//! resources (`rustcms make:resource`), pages (`make:settings`) and labels.

use rustcms::prelude::*;

pub fn provider() -> AdminProvider {
    AdminProvider::new()
        .brand("RDockPanel")
        // `/admin` opens the Dashboard; the resources stay reachable (their routes
        // serve the React pages) but have no sidebar entry (`navigation(false)`).
        .home("dashboard")
        // Light by default, as EasyPanel; not the operating system's scheme.
        // Success and warning stay neutral (RustCMS 0.10.2 made them green and
        // amber): the kit's warning alerts tint their text with it, and the
        // panel keeps status colors to dots and icons (docs/ui-guidelines.md).
        .theme(
            rustcms::admin::Theme::new()
                .color_scheme(rustcms::admin::ColorScheme::Light)
                .color("success", "#3f3f46")
                .color("warning", "#52525b")
                .dark(|p| p.color("success", "#d4d4d8").color("warning", "#d4d4d8")),
        )
        // Settings → Branding, while the panel runs.
        .branding(crate::services::branding_service::PanelBranding)
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
        .resource::<crate::admin::resources::cert_resolver_resource::CertResolverResource>()
        .resource::<crate::admin::resources::notification_channel_resource::NotificationChannelResource>()
        .resource::<crate::admin::resources::service_redirect_resource::ServiceRedirectResource>()
        .resource::<crate::admin::resources::service_script_resource::ServiceScriptResource>()
        // rustcms:resources
        // The apps interface first (React, host API v2).
        .page::<crate::admin::pages::dashboard_page::DashboardPage>()
        .page::<crate::admin::pages::domains_page::DomainsPage>()
        .page::<crate::admin::pages::firewall_page::FirewallPage>()
        .page::<crate::admin::pages::actions_page::ActionsPage>()
        .page::<crate::admin::pages::settings_page::SettingsPage>()
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
        .extension_script_v2(
            "dashboard",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/resources/admin/js/rdockpanel/dashboard.js"
            )),
            rustcms::admin::Requires {
                react: "19.3",
                react_query: "5.104",
                ui: 2,
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
