//! SeaORM entities. `rustcms make:model <Name>` adds modules here.

pub mod action;
pub mod backup_run;
pub mod branding_image;
pub mod cert_resolver;
pub mod certificate;
pub mod container_import;
pub mod container_import_claim;
pub mod container_import_file;
pub mod container_import_item;
pub mod container_import_volume;
pub mod database_backup;
pub mod deployment;
pub mod firewall_blocked;
pub mod firewall_opening;
pub mod firewall_transaction;
pub mod git_provider;
pub mod notification_channel;
pub mod panel_setting;
pub mod project;
pub mod project_member;
pub mod service;
pub mod service_basic_auth;
pub mod service_domain;
pub mod service_logo;
pub mod service_mount;
pub mod service_port;
pub mod service_redirect;
pub mod service_script;
pub mod storage_provider;
pub mod tunnel;
// rustcms:models
