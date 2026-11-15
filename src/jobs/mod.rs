//! Background jobs. `rustcms make:job <Name>` adds modules here.

pub mod import_discover;
pub mod import_step;
pub mod publish_routes;
pub mod pull_image;
pub mod run_backup;
pub mod run_deployment;
// rustcms:jobs
