//! Services, jobs, seeders and console commands of the application
//! (`rustcms make:job`, `make:seeder`, `make:command` add them here).

use rustcms::async_trait;
use rustcms::prelude::*;

pub struct AppServiceProvider;

#[async_trait]
impl ServiceProvider for AppServiceProvider {
    fn register(&self, app: &mut AppBuilder) {
        // Tests bind fakes over these (`bind` replaces by type).
        crate::services::deploy_rules_service::configure(
            crate::services::deploy_rules_service::PanelConfig::from_config(app.config()),
        );
        app.bind(crate::services::docker_service::Docker::from_env());
        app.bind(crate::services::traefik_service::Traefik::from_config(
            crate::services::deploy_rules_service::config(),
        ));
        app.bind(crate::services::system_stats_service::SystemStats::from_env());
        let shutdown = app.shutdown();
        app.bind(crate::services::live_service::Live::new(shutdown));
        // `rustcms db:seed` runs this one (it calls the others).
        app.seeder(crate::database::seeders::DatabaseSeeder);
        // rustcms:seeders
        app.job::<crate::jobs::pull_image::PullImage>();
        app.job::<crate::jobs::run_deployment::RunDeployment>();
        app.job::<crate::jobs::publish_routes::PublishRoutes>();
        // rustcms:jobs
        // rustcms:commands
    }

    async fn boot(&self, state: &AppState) -> anyhow::Result<()> {
        // Deployments whose run died: interrupted, their services released.
        match crate::services::deployment_service::recover_interrupted(state.db()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(n, "deployments interrupted by a stop"),
            Err(err) => tracing::debug!(%err, "interrupted deployments not checked"),
        }
        // Then every minute: a run whose job timed out or died stops renewing.
        crate::services::deployment_service::reap_every_minute(state);
        // Traefik's routes, written again from the database.
        if let Some(traefik) = state.service::<crate::services::traefik_service::Traefik>() {
            let options = crate::services::deployment_service::deploy_options(0);
            match crate::services::traefik_service::sync_all(state.db(), &traefik, &options).await {
                Ok(n) => tracing::info!(n, dir = %traefik.dir.display(), "routes written"),
                Err(err) => tracing::debug!(%err, "routes not written"),
            }
        }
        use crate::http::controllers::docker_controller::TERMINAL_SESSION;
        use crate::services::action_service::interrupted_terminals;
        use crate::services::docker_service::Docker;
        // Terminals cut by a stop: their shells may still run in the containers.
        match interrupted_terminals(state.db(), TERMINAL_SESSION).await {
            Ok(shells) if !shells.is_empty() => {
                if let Some(docker) = state.service::<Docker>() {
                    for (container, token) in shells {
                        if let Err(err) = docker.end_exec(&container, &token).await {
                            tracing::warn!(%err, container, "could not end an interrupted terminal");
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(err) => tracing::debug!(%err, "interrupted terminals not checked"),
        }
        // Before migrations ran (a fresh install's `migrate`) there is no table yet.
        match crate::services::action_service::reap_stale(state.db()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(n, "marked abandoned actions failed"),
            Err(err) => tracing::debug!(%err, "abandoned actions not checked"),
        }
        Ok(())
    }
}
