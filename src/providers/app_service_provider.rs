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
        let shutdown = app.shutdown();
        app.bind(crate::services::live_service::Live::new(shutdown));
        // `rustcms db:seed` runs this one (it calls the others).
        app.seeder(crate::database::seeders::DatabaseSeeder);
        // rustcms:seeders
        app.job::<crate::jobs::pull_image::PullImage>();
        // rustcms:jobs
        // rustcms:commands
    }

    async fn boot(&self, state: &AppState) -> anyhow::Result<()> {
        // Before migrations ran (a fresh install's `migrate`) there is no table yet.
        match crate::services::action_service::reap_stale(state.db()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(n, "marked abandoned actions failed"),
            Err(err) => tracing::debug!(%err, "abandoned actions not checked"),
        }
        Ok(())
    }
}
