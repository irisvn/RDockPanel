//! Services, jobs, seeders and console commands of the application
//! (`rustcms make:job`, `make:seeder`, `make:command` add them here).

use rustcms::async_trait;
use rustcms::prelude::*;

pub struct AppServiceProvider;

#[async_trait]
impl ServiceProvider for AppServiceProvider {
    fn register(&self, app: &mut AppBuilder) {
        // Tests bind fakes over these (`bind` replaces by type).
        app.bind(crate::services::docker_service::Docker::from_env());
        app.bind(crate::services::system_stats_service::SystemStats::from_env());
        // `rustcms db:seed` runs this one (it calls the others).
        app.seeder(crate::database::seeders::DatabaseSeeder);
        // rustcms:seeders
        // rustcms:jobs
        // rustcms:commands
    }
}
