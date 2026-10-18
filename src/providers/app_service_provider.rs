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
        app.bind(crate::services::template_service::Templates::new(
            crate::services::deploy_rules_service::config()
                .templates_url
                .clone(),
        ));
        app.bind(crate::services::traefik_service::Traefik::from_config(
            crate::services::deploy_rules_service::config(),
        ));
        app.bind(crate::services::system_stats_service::SystemStats::from_env());
        let shutdown = app.shutdown();
        app.bind(crate::services::live_service::Live::new(shutdown));
        // Sealed outside the admin's fields: found by `rustcms key:reencrypt`.
        app.encrypted_column::<crate::models::deployment::Entity>(
            crate::models::deployment::Column::Env,
            rustcms::crypto::Binding::Column,
        );
        app.encrypted_column::<crate::models::deployment::Entity>(
            crate::models::deployment::Column::Files,
            rustcms::crypto::Binding::Column,
        );
        app.encrypted_column::<crate::models::notification_channel::Entity>(
            crate::models::notification_channel::Column::Secret,
            rustcms::crypto::Binding::Column,
        );
        app.encrypted_column::<crate::models::storage_provider::Entity>(
            crate::models::storage_provider::Column::SecretKey,
            rustcms::crypto::Binding::Column,
        );
        app.encrypted_column::<crate::models::service::Entity>(
            crate::models::service::Column::WebhookSecret,
            rustcms::crypto::Binding::Column,
        );
        app.encrypted_column::<crate::models::service::Entity>(
            crate::models::service::Column::DeployKey,
            rustcms::crypto::Binding::Column,
        );
        // `rustcms db:seed` runs this one (it calls the others).
        app.seeder(crate::database::seeders::DatabaseSeeder);
        // rustcms:seeders
        app.job::<crate::jobs::pull_image::PullImage>();
        app.job::<crate::jobs::run_deployment::RunDeployment>();
        app.job::<crate::jobs::publish_routes::PublishRoutes>();
        app.job::<crate::jobs::run_backup::RunBackup>();
        // Backups whose cron fired: queued (each configuration's own
        // schedule; the slot claimed in the database).
        app.schedule(
            rustcms::schedule::Schedule::every(
                "database-backups",
                std::time::Duration::from_secs(60),
            )
            .run(|state| async move {
                let n = crate::services::backup_service::tick(&state).await?;
                if n > 0 {
                    tracing::info!(n, "scheduled backups queued");
                }
                Ok(())
            }),
        );
        // Deployments whose run stopped renewing its lease (`serve` only;
        // in the process, never queued behind long pulls or deployments).
        app.schedule(
            rustcms::schedule::Schedule::every(
                "reap-deployments",
                std::time::Duration::from_secs(60),
            )
            .run(|state| async move {
                let n =
                    crate::services::deployment_service::recover_interrupted(state.db()).await?;
                if n > 0 {
                    tracing::warn!(n, "deployments interrupted (their run stopped)");
                }
                // Pushes kept for a deployment cut short: deployed now.
                crate::services::deployment_service::drain_pending(&state).await?;
                // Scripts whose request is gone: killed, then finished (a
                // kill that fails is tried again next time).
                use crate::http::controllers::apps_controller::{SCRIPT_ACTION, SCRIPT_MARK};
                let left = crate::services::action_service::abandoned_scripts(
                    state.db(),
                    SCRIPT_ACTION,
                    SCRIPT_MARK,
                    chrono::Duration::minutes(5),
                )
                .await?;
                let docker = state.service::<crate::services::docker_service::Docker>();
                for (row, mark) in left {
                    let killed = match (&mark, &docker) {
                        (Some((container, mark)), Some(docker)) => {
                            docker.engine().kill_marked(container, mark).await
                        }
                        (None, _) => Ok(()),
                        (Some(_), None) => continue,
                    };
                    match killed {
                        Ok(()) => {
                            crate::services::action_service::finish(
                                state.db(),
                                row,
                                Some("interrupted: its request ended before it".to_owned()),
                                None,
                            )
                            .await?;
                        }
                        Err(err) => tracing::warn!(%err, "an abandoned script not killed yet"),
                    }
                }
                Ok(())
            }),
        );
        // The networks of deleted projects: Traefik detached, then removed.
        app.schedule(
            rustcms::schedule::Schedule::every(
                "orphan-networks",
                std::time::Duration::from_secs(300),
            )
            .run(|state| async move {
                use crate::services::docker_service::Docker;
                use crate::services::traefik_service::{Traefik, remove_orphan_networks};
                if let Some(docker) = state.service::<Docker>() {
                    let traefik = state.service::<Traefik>();
                    remove_orphan_networks(state.db(), &docker, traefik.as_deref()).await?;
                }
                Ok(())
            }),
        );
        // A full disk, said to the channels that asked (each at most every 6 h).
        app.schedule(
            rustcms::schedule::Schedule::every("disk-check", std::time::Duration::from_secs(300))
                .run(|state| async move {
                    crate::services::notify_service::check_disk(&state);
                    Ok(())
                }),
        );
        // rustcms:jobs
        // rustcms:commands
    }

    async fn boot(&self, state: &AppState) -> anyhow::Result<()> {
        // bollard logs each request's body at debug (through the `log`
        // crate): a service spec holds its environment's values. Whatever
        // RUST_LOG says, `log` records stop at info (the panel's own logs are
        // `tracing`'s).
        log::set_max_level(log::LevelFilter::Info.min(log::max_level()));
        // Deployments whose run died: interrupted, their services released.
        match crate::services::deployment_service::recover_interrupted(state.db()).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(n, "deployments interrupted by a stop"),
            Err(err) => tracing::debug!(%err, "interrupted deployments not checked"),
        }
        match crate::services::backup_service::recover_interrupted(
            state.db(),
            state
                .service::<crate::services::docker_service::Docker>()
                .as_deref(),
        )
        .await
        {
            Ok(0) => {}
            Ok(n) => tracing::info!(n, "backups interrupted by a stop"),
            Err(err) => tracing::debug!(%err, "interrupted backups not checked"),
        }
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
