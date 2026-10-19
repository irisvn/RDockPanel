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
        app.encrypted_column::<crate::models::certificate::Entity>(
            crate::models::certificate::Column::Key,
            rustcms::crypto::Binding::Column,
        );
        app.encrypted_column::<crate::models::cert_resolver::Entity>(
            crate::models::cert_resolver::Column::Token,
            rustcms::crypto::Binding::Column,
        );
        app.encrypted_column::<crate::models::git_provider::Entity>(
            crate::models::git_provider::Column::Token,
            rustcms::crypto::Binding::Column,
        );
        app.bind(crate::services::github_service::GitHub::default());
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
        // Traefik's certificate resolvers, given again when an installer run
        // replaced its arguments (`cert_service::reconcile`).
        app.schedule(
            rustcms::schedule::Schedule::every(
                "traefik-resolvers",
                std::time::Duration::from_secs(120),
            )
            .run(|state| async move {
                use crate::services::cert_service::{plan, plan_key, reconcile, resolvers};
                use crate::services::docker_service::Docker;
                use crate::services::traefik_service::Traefik;
                let (Some(docker), Some(traefik)) =
                    (state.service::<Docker>(), state.service::<Traefik>())
                else {
                    return Ok(());
                };
                let resolvers = resolvers(state.db(), state.keyring())
                    .await
                    .map_err(rustcms::AppError::internal)?;
                let plan = plan(&resolvers, &plan_key(state.keyring()));
                if reconcile(&docker, &traefik, &plan).await? {
                    tracing::info!("Traefik's certificate resolvers given again");
                }
                Ok(())
            }),
        );
        // Uploaded certificates expiring: said to the channels (each warning once).
        app.schedule(
            rustcms::schedule::Schedule::every(
                "certificate-expiry",
                std::time::Duration::from_secs(6 * 60 * 60),
            )
            .run(|state| async move {
                crate::services::certificate_service::check_expiry(&state).await
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
        // Running services and Traefik failing or back; a newer panel; a
        // sign-in from a new address (each said once: `alert_service`).
        app.schedule(
            rustcms::schedule::Schedule::every(
                "health-alerts",
                std::time::Duration::from_secs(120),
            )
            .run(|state| async move { crate::services::alert_service::check_health(&state).await }),
        );
        app.schedule(
            rustcms::schedule::Schedule::every(
                "update-alerts",
                std::time::Duration::from_secs(6 * 60 * 60),
            )
            .run(|state| async move { crate::services::alert_service::check_update(&state).await }),
        );
        app.schedule(
            rustcms::schedule::Schedule::every("login-alerts", std::time::Duration::from_secs(60))
                .run(|state| async move { crate::services::alert_service::check_logins(&state).await }),
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
        crate::services::deploy_service::set_instance(state.keyring());
        // Members of projects deleted since: forgotten.
        match crate::services::access_service::forget_deleted_projects(state).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(n, "memberships of deleted projects removed"),
            Err(err) => tracing::debug!(%err, "memberships not checked"),
        }
        // The public address Refresh IP found (else the installer's).
        if let Err(err) = crate::services::system_service::load_public_ip(state.db()).await {
            tracing::debug!(%err, "public address not read");
        }
        // A restart of the panel or a reboot it asked for: it came back
        // (the server only; a console command is not the panel).
        if std::env::args().nth(1).as_deref() == Some("serve") {
            match crate::services::action_service::came_back(
                state.db(),
                &crate::services::system_service::DISRUPTIVE,
            )
            .await
            {
                Ok(0) => {}
                Ok(n) => tracing::info!(n, "restart or reboot done"),
                Err(err) => tracing::debug!(%err, "restarts not checked"),
            }
        }
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
            // The administrators' middlewares, the uploaded certificates too.
            if let Err(err) =
                crate::services::traefik_settings_service::sync_middlewares(state.db(), &traefik)
                    .await
            {
                tracing::warn!(%err, "middlewares not written");
            }
            if let Err(err) =
                crate::services::certificate_service::sync(state.db(), state.keyring(), &traefik)
                    .await
            {
                tracing::warn!(%err, "certificates not written");
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
