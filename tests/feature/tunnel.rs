//! Settings → Cloudflare Tunnel: cloudflared runs with the token as a Swarm
//! secret (never an argument or variable), Traefik gets the tunnel's
//! entrypoint and network; administrators only.

use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::docker_service::DockerApi;
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestDb, db_test};
use serde_json::json;

use crate::support::{self, fake_docker::FakeDocker};

/// A token as Cloudflare gives one (base64 of JSON), never a real one.
const TOKEN: &str = "eyJhIjoiMDEyMzQ1Njc4OWFiY2RlZjAxMjM0NTY3ODlhYmNkZWYiLCJ0IjoiMDEyMzQ1NjctODlhYi1jZGVmLTAxMjMtNDU2Nzg5YWJjZGVmIiwicyI6ImMyVmpjbVYwTFhObFkzSmxkQzF6WldOeVpYUT0ifQ==";

fn spec(fake: &FakeDocker, name: &str) -> Option<serde_json::Value> {
    fake.services
        .lock()
        .unwrap()
        .get(name)
        .map(|s| serde_json::to_value(s.spec.clone().unwrap()).unwrap())
}

db_test!(
    the_tunnel_runs_cloudflared_with_its_token_as_a_secret,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let url = |p: &str| app.admin_api(&format!("x/apps/tunnel{p}"));

        // Administrators only.
        let user = support::user_with(&app, "dev", &["projects.view_any"]).await;
        let dev = app.acting_as(&user).await;
        dev.get(&url("")).await.assert_status(StatusCode::FORBIDDEN);
        dev.post_json(&url("/enable"), &json!({"token": TOKEN}))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let status = admin.get(&url("")).await;
        status.assert_status(StatusCode::OK);
        assert_eq!(status.json()["enabled"], false);
        assert_eq!(status.json()["token_set"], false);

        // A token is required, and must look like one.
        admin
            .post_json(&url("/enable"), &json!({"token": null}))
            .await
            .assert_validation_error("token");
        admin
            .post_json(&url("/enable"), &json!({"token": "not a token; rm -rf /"}))
            .await
            .assert_validation_error("token");
        // Base64 of anything else: no tunnel token either.
        admin
            .post_json(&url("/enable"), &json!({"token": "A".repeat(60)}))
            .await
            .assert_validation_error("token");
        assert!(spec(&fake, "rdp-cloudflared").is_none());

        // On: cloudflared reads the token from a secret.
        let on = admin
            .post_json(&url("/enable"), &json!({"token": TOKEN}))
            .await;
        on.assert_status(StatusCode::OK);
        assert_eq!(on.json()["enabled"], true);
        assert_eq!(on.json()["token_set"], true);
        let cloudflared = spec(&fake, "rdp-cloudflared").unwrap();
        let text = cloudflared.to_string();
        assert!(!text.contains(TOKEN), "{text}");
        let container = &cloudflared["TaskTemplate"]["ContainerSpec"];
        assert!(container["Args"].to_string().contains("--token-file"));
        assert!(container["Env"].is_null());
        let secret = container["Secrets"][0]["SecretName"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(fake.secrets.lock().unwrap()[&secret].1, TOKEN.as_bytes());
        // Readable by cloudflared's user (its image's `nonroot`).
        assert_eq!(container["Secrets"][0]["File"]["UID"], "65532");
        // Kept sealed, never in clear.
        let kept = rdockpanel::models::panel_setting::get(app.state().db(), "tunnel.token")
            .await
            .unwrap()
            .unwrap();
        assert!(!kept.contains(TOKEN));
        // Traefik: the tunnel's entrypoint (once) and network.
        let traefik = spec(&fake, "rdp-traefik").unwrap();
        let args = traefik["TaskTemplate"]["ContainerSpec"]["Args"].to_string();
        assert!(
            args.contains("--entrypoints.tunnel.address=:8081"),
            "{args}"
        );
        assert!(
            traefik["TaskTemplate"]["Networks"]
                .to_string()
                .contains("rdp-tunnel")
        );

        // Again without a token: the one kept; the same secret.
        admin
            .post_json(&url("/enable"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        let again = spec(&fake, "rdp-cloudflared").unwrap();
        assert_eq!(
            again["TaskTemplate"]["ContainerSpec"]["Secrets"][0]["SecretName"],
            secret
        );
        let traefik = spec(&fake, "rdp-traefik").unwrap();
        let args: Vec<String> =
            serde_json::from_value(traefik["TaskTemplate"]["ContainerSpec"]["Args"].clone())
                .unwrap();
        assert_eq!(
            args.iter()
                .filter(|a| a.starts_with("--entrypoints.tunnel.address"))
                .count(),
            1
        );

        // Off: cloudflared removed, the token kept.
        let off = admin.post_json(&url("/disable"), &json!({})).await;
        off.assert_status(StatusCode::OK);
        assert_eq!(off.json()["enabled"], false);
        assert_eq!(off.json()["token_set"], true);
        assert!(spec(&fake, "rdp-cloudflared").is_none());
    }
);

db_test!(
    an_installed_traefik_keeps_its_tunnel_entrypoint_as_it_is,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        // As the installer starts it: the entrypoint there before any tunnel.
        let installed = [
            "--providers.file.directory=/etc/traefik/dynamic",
            "--entrypoints.tunnel.address=:8081",
            "--entrypoints.tunnel.http.aliasHeadersStrategy=delete",
        ];
        {
            let mut services = fake.services.lock().unwrap();
            let traefik = services.get_mut("rdp-traefik").unwrap();
            let spec = traefik.spec.as_mut().unwrap();
            spec.task_template
                .as_mut()
                .unwrap()
                .container_spec
                .as_mut()
                .unwrap()
                .args = Some(installed.iter().map(|a| a.to_string()).collect());
        }
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        admin
            .post_json(
                &app.admin_api("x/apps/tunnel/enable"),
                &json!({"token": TOKEN}),
            )
            .await
            .assert_status(StatusCode::OK);
        let args = || -> Vec<String> {
            let traefik = spec(&fake, "rdp-traefik").unwrap();
            serde_json::from_value(traefik["TaskTemplate"]["ContainerSpec"]["Args"].clone())
                .unwrap()
        };
        // Kept as they are, the tunnel's network trusted (cloudflared names
        // the visitor; a project's container is not on it).
        let subnets = fake.network_subnets("rdp-tunnel").await.unwrap();
        let mut want: Vec<String> = installed.iter().map(|a| a.to_string()).collect();
        want.push(format!(
            "--entrypoints.tunnel.forwardedHeaders.trustedIPs={}",
            subnets.join(",")
        ));
        assert_eq!(args(), want);
        // Again: nothing changes.
        let version = fake.services.lock().unwrap()["rdp-traefik"].version.clone();
        admin
            .post_json(&app.admin_api("x/apps/tunnel/enable"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(args(), want);
        assert_eq!(
            fake.services.lock().unwrap()["rdp-traefik"].version,
            version
        );
    }
);

db_test!(
    an_older_cloudflared_is_run_again_as_pinned,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        admin
            .post_json(
                &app.admin_api("x/apps/tunnel/enable"),
                &json!({"token": TOKEN}),
            )
            .await
            .assert_status(StatusCode::OK);
        let image = |fake: &FakeDocker| {
            spec(fake, "rdp-cloudflared").unwrap()["TaskTemplate"]["ContainerSpec"]["Image"]
                .as_str()
                .unwrap()
                .to_owned()
        };
        assert_eq!(image(&fake), rdockpanel::services::tunnel_service::IMAGE);
        // An older panel's cloudflared (Cloudflare stops supporting old ones).
        {
            let mut services = fake.services.lock().unwrap();
            let service = services.get_mut("rdp-cloudflared").unwrap();
            let container = service
                .spec
                .as_mut()
                .unwrap()
                .task_template
                .as_mut()
                .unwrap()
                .container_spec
                .as_mut()
                .unwrap();
            container.image = Some("cloudflare/cloudflared:2025.9.1@sha256:00".to_owned());
        }
        // At boot: run again with the pinned one (in the background).
        rdockpanel::services::tunnel_service::reconcile(app.state());
        for _ in 0..100 {
            if image(&fake) == rdockpanel::services::tunnel_service::IMAGE {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(image(&fake), rdockpanel::services::tunnel_service::IMAGE);
    }
);

/// A second Cloudflare account's token (base64 of JSON), never a real one.
const OTHER_TOKEN: &str = "eyJhIjoiZmVkY2JhOTg3NjU0MzIxMGZlZGNiYTk4NzY1NDMyMTAiLCJ0IjoiNzY1NDMyMTAtYmE5OC1mZWRjLTMyMTAtZmVkY2JhOTg3NjU0IiwicyI6ImIzUm9aWEl0YzJWamNtVjBMVzkwYUdWeUxYTmxZM0psZEE9PSJ9";

fn docker_of(
    app: &rustcms_testing::TestApp,
) -> std::sync::Arc<rdockpanel::services::docker_service::Docker> {
    app.state()
        .service::<rdockpanel::services::docker_service::Docker>()
        .unwrap()
}

fn secret_of(fake: &FakeDocker, service: &str) -> String {
    spec(fake, service).unwrap()["TaskTemplate"]["ContainerSpec"]["Secrets"][0]["SecretName"]
        .as_str()
        .unwrap()
        .to_owned()
}

db_test!(
    a_second_tunnel_runs_beside_the_first,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::tunnel_service;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let docker = docker_of(&app);
        tunnel_service::enable(app.state(), &docker, "default", Some(TOKEN))
            .await
            .unwrap();
        tunnel_service::enable(app.state(), &docker, "second", Some(OTHER_TOKEN))
            .await
            .unwrap();
        assert_eq!(
            tunnel_service::service_name("second"),
            "rdp-cloudflared-second"
        );
        let (first, second) = (
            secret_of(&fake, "rdp-cloudflared"),
            secret_of(&fake, "rdp-cloudflared-second"),
        );
        assert_ne!(first, second);
        let secrets = fake.secrets.lock().unwrap().clone();
        assert_eq!(secrets[&first].1, TOKEN.as_bytes());
        assert_eq!(secrets[&second].1, OTHER_TOKEN.as_bytes());
        // Both on the tunnel's network, which Traefik shares.
        for name in ["rdp-cloudflared", "rdp-cloudflared-second"] {
            assert_eq!(
                spec(&fake, name).unwrap()["TaskTemplate"]["Networks"][0]["Target"],
                "netid-rdp-tunnel"
            );
        }
        let running = tunnel_service::running_tokens(app.state()).await.unwrap();
        assert_eq!(
            running,
            vec![
                ("default".to_owned(), TOKEN.to_owned()),
                ("second".to_owned(), OTHER_TOKEN.to_owned())
            ]
        );
        // Listed with their state, never their token.
        let list = tunnel_service::list(app.state(), &docker).await.unwrap();
        assert_eq!(list.as_array().unwrap().len(), 2);
        assert!(!list.to_string().contains(OTHER_TOKEN));
        assert_eq!(list[1]["name"], "second");
        assert_eq!(list[1]["enabled"], true);
        // A name of letters, digits and dashes only.
        let bad = tunnel_service::enable(app.state(), &docker, "Bad Name", Some(OTHER_TOKEN)).await;
        assert!(bad.is_err());
    }
);

db_test!(
    the_same_token_cannot_be_added_twice,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let docker = docker_of(&app);
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        admin
            .post_json(
                &app.admin_api("x/apps/tunnel/enable"),
                &json!({"token": TOKEN}),
            )
            .await
            .assert_status(StatusCode::OK);
        let again = rdockpanel::services::tunnel_service::enable(
            app.state(),
            &docker,
            "second",
            Some(TOKEN),
        )
        .await;
        let Err(rustcms::AppError::Validation(errors)) = again else {
            panic!("{again:?}");
        };
        assert!(serde_json::to_string(&errors).unwrap().contains("token"));
        assert!(spec(&fake, "rdp-cloudflared-second").is_none());
    }
);

db_test!(
    the_existing_tunnel_is_migrated_as_default,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::tunnel_service;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        // A token this APP_KEY cannot open: no row (a later boot tries again).
        let foreign = rustcms::crypto::Encrypted::seal(
            app.state().keyring(),
            &rustcms::crypto::Context::column("elsewhere", "token"),
            &TOKEN.to_owned(),
        )
        .unwrap();
        rdockpanel::models::panel_setting::set(
            app.state().db(),
            "tunnel.token",
            foreign.envelope(),
        )
        .await
        .unwrap();
        tunnel_service::load(app.state()).await.unwrap();
        let none = tunnel_service::list(app.state(), &docker_of(&app))
            .await
            .unwrap();
        assert_eq!(none, json!([]));
        // As an older panel kept it: the settings only.
        let sealed = rustcms::crypto::Encrypted::seal(
            app.state().keyring(),
            &rustcms::crypto::Context::column("panel_settings", "tunnel.token"),
            &TOKEN.to_owned(),
        )
        .unwrap();
        let db = app.state().db();
        rdockpanel::models::panel_setting::set(db, "tunnel.token", sealed.envelope())
            .await
            .unwrap();
        rdockpanel::models::panel_setting::set(db, "tunnel.enabled", "true")
            .await
            .unwrap();
        tunnel_service::load(app.state()).await.unwrap();
        // Twice: one row.
        tunnel_service::load(app.state()).await.unwrap();
        let docker = docker_of(&app);
        let list = tunnel_service::list(app.state(), &docker).await.unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["name"], "default");
        assert_eq!(list[0]["enabled"], true);
        assert!(tunnel_service::enabled(app.state()));
        assert_eq!(
            tunnel_service::running_tokens(app.state()).await.unwrap(),
            vec![("default".to_owned(), TOKEN.to_owned())]
        );
        // Run with the token kept: its service's name as before.
        tunnel_service::enable(app.state(), &docker, "default", None)
            .await
            .unwrap();
        assert_eq!(tunnel_service::service_name("default"), "rdp-cloudflared");
        assert!(spec(&fake, "rdp-cloudflared").is_some());
    }
);

db_test!(
    removing_one_tunnel_keeps_the_other_running,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::tunnel_service;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let docker = docker_of(&app);
        tunnel_service::enable(app.state(), &docker, "default", Some(TOKEN))
            .await
            .unwrap();
        tunnel_service::enable(app.state(), &docker, "second", Some(OTHER_TOKEN))
            .await
            .unwrap();
        let kept = secret_of(&fake, "rdp-cloudflared");
        let gone = secret_of(&fake, "rdp-cloudflared-second");
        tunnel_service::remove(app.state(), &docker, "second")
            .await
            .unwrap();
        assert!(spec(&fake, "rdp-cloudflared-second").is_none());
        assert!(spec(&fake, "rdp-cloudflared").is_some());
        let secrets = fake.secrets.lock().unwrap().clone();
        assert!(secrets.contains_key(&kept));
        assert!(!secrets.contains_key(&gone));
        assert!(tunnel_service::enabled(app.state()));
        assert_eq!(
            tunnel_service::running_tokens(app.state()).await.unwrap(),
            vec![("default".to_owned(), TOKEN.to_owned())]
        );
        // Turning the default off leaves no tunnel on.
        tunnel_service::disable(app.state(), &docker, "default")
            .await
            .unwrap();
        assert!(!tunnel_service::enabled(app.state()));
        assert!(spec(&fake, "rdp-cloudflared").is_none());
    }
);

db_test!(
    a_rolled_back_release_s_tunnel_settings_are_taken_back,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::tunnel_service;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let docker = docker_of(&app);
        tunnel_service::enable(app.state(), &docker, "default", Some(TOKEN))
            .await
            .unwrap();
        // The older release (rolled back to) turned it off and set another token.
        let db = app.state().db();
        let other = rustcms::crypto::Encrypted::seal(
            app.state().keyring(),
            &rustcms::crypto::Context::column("panel_settings", "tunnel.token"),
            &OTHER_TOKEN.to_owned(),
        )
        .unwrap();
        rdockpanel::models::panel_setting::set(db, "tunnel.token", other.envelope())
            .await
            .unwrap();
        rdockpanel::models::panel_setting::set(db, "tunnel.enabled", "false")
            .await
            .unwrap();
        tunnel_service::load(app.state()).await.unwrap();
        let list = tunnel_service::list(app.state(), &docker).await.unwrap();
        assert_eq!(list[0]["enabled"], false);
        assert!(!tunnel_service::enabled(app.state()));
        // On again there: run with its token.
        rdockpanel::models::panel_setting::set(db, "tunnel.enabled", "true")
            .await
            .unwrap();
        tunnel_service::load(app.state()).await.unwrap();
        assert_eq!(
            tunnel_service::running_tokens(app.state()).await.unwrap(),
            vec![("default".to_owned(), OTHER_TOKEN.to_owned())]
        );
    }
);

db_test!(
    an_unknown_tunnel_is_not_found,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::tunnel_service;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let docker = docker_of(&app);
        for name in ["missing", "Bad Name", "../x"] {
            assert!(matches!(
                tunnel_service::status(app.state(), &docker, name).await,
                Err(rustcms::AppError::NotFound)
            ));
            assert!(matches!(
                tunnel_service::disable(app.state(), &docker, name).await,
                Err(rustcms::AppError::NotFound)
            ));
        }
        // `default` before it is made: off (the old routes).
        tunnel_service::status(app.state(), &docker, "default")
            .await
            .unwrap();
        tunnel_service::disable(app.state(), &docker, "default")
            .await
            .unwrap();
    }
);

db_test!(
    a_tunnel_is_added_listed_and_removed_over_the_api,
    migrate = Migrator,
    |db: TestDb| async move {
        use rustcms::axum::http::Method;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let url = |p: &str| app.admin_api(&format!("x/apps/tunnels{p}"));

        // A name and a token, each checked.
        admin
            .post_json(&url(""), &json!({"name": "Bad Name", "token": OTHER_TOKEN}))
            .await
            .assert_validation_error("name");
        admin
            .post_json(&url(""), &json!({"name": "second", "token": "nope"}))
            .await
            .assert_validation_error("token");
        admin
            .post_json(&url(""), &json!({"name": "second"}))
            .await
            .assert_validation_error("token");

        let added = admin
            .post_json(&url(""), &json!({"name": "second", "token": OTHER_TOKEN}))
            .await;
        added.assert_status(StatusCode::OK);
        assert_eq!(added.json()["name"], "second");
        assert_eq!(added.json()["enabled"], true);
        assert!(spec(&fake, "rdp-cloudflared-second").is_some());
        // A name is one tunnel.
        admin
            .post_json(&url(""), &json!({"name": "second", "token": TOKEN}))
            .await
            .assert_validation_error("name");

        let list = admin.get(&url("")).await;
        list.assert_status(StatusCode::OK);
        let tunnels = list.json()["tunnels"].as_array().unwrap().clone();
        assert_eq!(tunnels.len(), 1);
        assert_eq!(tunnels[0]["name"], "second");
        assert_eq!(tunnels[0]["service"], true);
        assert!(tunnels[0]["origin"].as_str().unwrap().ends_with(":8081"));

        let off = admin.post_json(&url("/second/disable"), &json!({})).await;
        off.assert_status(StatusCode::OK);
        assert_eq!(off.json()["enabled"], false);
        assert!(spec(&fake, "rdp-cloudflared-second").is_none());
        let on = admin.post_json(&url("/second/enable"), &json!({})).await;
        on.assert_status(StatusCode::OK);
        assert_eq!(on.json()["enabled"], true);
        assert!(spec(&fake, "rdp-cloudflared-second").is_some());
        // An unknown tunnel: not found.
        admin
            .post_json(&url("/missing/enable"), &json!({"token": TOKEN}))
            .await
            .assert_status(StatusCode::NOT_FOUND);

        let deleted = admin.send(Method::DELETE, &url("/second"), None).await;
        deleted.assert_status(StatusCode::OK);
        assert_eq!(deleted.json(), json!({"deleted": "second"}));
        assert!(spec(&fake, "rdp-cloudflared-second").is_none());
        let list = admin.get(&url("")).await;
        assert_eq!(list.json()["tunnels"], json!([]));
        admin
            .send(Method::DELETE, &url("/second"), None)
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
);

db_test!(
    deleting_a_tunnel_stops_only_its_service,
    migrate = Migrator,
    |db: TestDb| async move {
        use rustcms::axum::http::Method;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let url = |p: &str| app.admin_api(&format!("x/apps/tunnels{p}"));
        for (name, token) in [("first", TOKEN), ("second", OTHER_TOKEN)] {
            admin
                .post_json(&url(""), &json!({"name": name, "token": token}))
                .await
                .assert_status(StatusCode::OK);
        }
        admin
            .send(Method::DELETE, &url("/first"), None)
            .await
            .assert_status(StatusCode::OK);
        assert!(spec(&fake, "rdp-cloudflared-first").is_none());
        assert!(spec(&fake, "rdp-cloudflared-second").is_some());
        let second = secret_of(&fake, "rdp-cloudflared-second");
        assert!(fake.secrets.lock().unwrap().contains_key(&second));
        let list = admin.get(&url("")).await;
        let names: Vec<_> = list.json()["tunnels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].clone())
            .collect();
        assert_eq!(names, vec![json!("second")]);
    }
);

db_test!(
    a_tunnel_route_needs_system_manage,
    migrate = Migrator,
    |db: TestDb| async move {
        use rustcms::axum::http::Method;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let url = |p: &str| app.admin_api(&format!("x/apps/tunnels{p}"));
        let user = support::user_with(&app, "dev", &["projects.view_any", "docker.manage"]).await;
        let dev = app.acting_as(&user).await;
        let body = json!({"name": "second", "token": OTHER_TOKEN});
        for (method, path) in [
            (Method::GET, ""),
            (Method::POST, ""),
            (Method::POST, "/second/enable"),
            (Method::POST, "/second/disable"),
            (Method::DELETE, "/second"),
        ] {
            dev.send(method, &url(path), Some(&body))
                .await
                .assert_status(StatusCode::FORBIDDEN);
        }
        app.client()
            .get(&url(""))
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
        assert!(spec(&fake, "rdp-cloudflared-second").is_none());
    }
);

db_test!(
    the_tunnel_list_never_holds_a_token,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let url = |p: &str| app.admin_api(&format!("x/apps/tunnels{p}"));
        for (name, token) in [("first", TOKEN), ("second", OTHER_TOKEN)] {
            let added = admin
                .post_json(&url(""), &json!({"name": name, "token": token}))
                .await;
            added.assert_status(StatusCode::OK);
            assert!(!added.json().to_string().contains(token));
        }
        let list = admin.get(&url("")).await;
        list.assert_status(StatusCode::OK);
        let text = list.json().to_string();
        assert!(
            !text.contains(TOKEN) && !text.contains(OTHER_TOKEN),
            "{text}"
        );
        assert!(list.json()["tunnels"][0].get("token").is_none());
        // Nor the audit.
        use sea_orm::EntityTrait;
        let audit = rustcms::models::audit_log::Entity::find()
            .all(app.state().db())
            .await
            .unwrap();
        let audit = format!("{audit:?}");
        assert!(audit.contains("tunnel.create"), "{audit}");
        assert!(!audit.contains(TOKEN) && !audit.contains(OTHER_TOKEN));
    }
);

db_test!(
    a_change_is_audited_even_when_its_request_is_cut,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::tunnel_service;
        use rustcms::axum::http::Method;
        use rustcms::models::audit_log;
        use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let count = |event: &'static str| {
            let db = app.state().db().clone();
            async move {
                audit_log::Entity::find()
                    .filter(audit_log::Column::Event.eq(event))
                    .count(&db)
                    .await
                    .unwrap()
            }
        };
        let changes = [
            (
                Method::POST,
                "tunnels",
                json!({"name": "second", "token": OTHER_TOKEN}),
                "tunnel.create",
            ),
            (
                Method::POST,
                "tunnels/second/disable",
                json!({}),
                "tunnel.disable",
            ),
            (
                Method::POST,
                "tunnels/second/enable",
                json!({}),
                "tunnel.enable",
            ),
            (Method::DELETE, "tunnels/second", json!({}), "tunnel.remove"),
            (
                Method::POST,
                "tunnel/enable",
                json!({"token": TOKEN}),
                "tunnel.enable",
            ),
            (Method::POST, "tunnel/disable", json!({}), "tunnel.disable"),
        ];
        for (method, path, body, event) in changes {
            let before = count(event).await;
            // The change waits for another; the request is cut meanwhile.
            let busy = tunnel_service::exclusive().await;
            let cut = tokio::time::timeout(
                std::time::Duration::from_millis(500),
                admin.send(
                    method,
                    &app.admin_api(&format!("x/apps/{path}")),
                    Some(&body),
                ),
            )
            .await;
            assert!(cut.is_err(), "{path} answered while another change ran");
            drop(busy);
            // It still runs whole, its audit entry included.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while count(event).await == before {
                assert!(
                    std::time::Instant::now() < deadline,
                    "{path}: no {event} audited"
                );
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            // Its effect done before the next change.
            let _settled = tunnel_service::exclusive().await;
        }
        assert!(spec(&fake, "rdp-cloudflared-second").is_none());
        assert!(spec(&fake, "rdp-cloudflared").is_none());
    }
);

db_test!(
    a_tunnel_name_that_cannot_be_one_is_not_found,
    migrate = Migrator,
    |db: TestDb| async move {
        use rustcms::axum::http::Method;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        for name in ["Bad%20Name", "UPPER", "a.b", "missing"] {
            let url = |p: &str| app.admin_api(&format!("x/apps/tunnels/{name}{p}"));
            admin
                .post_json(&url("/enable"), &json!({"token": TOKEN}))
                .await
                .assert_status(StatusCode::NOT_FOUND);
            admin
                .post_json(&url("/disable"), &json!({}))
                .await
                .assert_status(StatusCode::NOT_FOUND);
            admin
                .send(Method::DELETE, &url(""), None)
                .await
                .assert_status(StatusCode::NOT_FOUND);
        }
        assert!(
            fake.services
                .lock()
                .unwrap()
                .keys()
                .all(|k| !k.starts_with("rdp-cloudflared"))
        );
    }
);

db_test!(
    replacing_the_token_of_a_tunnel_that_is_off_keeps_it_off,
    migrate = Migrator,
    |db: TestDb| async move {
        use rdockpanel::services::tunnel_service;
        use rustcms::axum::http::Method;
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let url = |p: &str| app.admin_api(&format!("x/apps/tunnels{p}"));
        admin
            .post_json(&url(""), &json!({"name": "second", "token": TOKEN}))
            .await
            .assert_status(StatusCode::OK);
        admin
            .post_json(&url("/second/disable"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        let replace = |token: &str| {
            let body = json!({ "token": token });
            let (admin, to) = (&admin, url("/second"));
            async move { admin.send(Method::PATCH, &to, Some(&body)).await }
        };
        replace("not a token")
            .await
            .assert_validation_error("token");
        let got = replace(OTHER_TOKEN).await;
        got.assert_status(StatusCode::OK);
        assert_eq!(got.json()["enabled"], false);
        assert!(!got.text().contains(OTHER_TOKEN));
        // Kept, still off: no cloudflared.
        let db = app.state().db();
        assert_eq!(
            tunnel_service::holder(app.state(), db, OTHER_TOKEN)
                .await
                .unwrap()
                .as_deref(),
            Some("second")
        );
        assert!(spec(&fake, "rdp-cloudflared-second").is_none());
        assert!(
            tunnel_service::running_tokens(app.state())
                .await
                .unwrap()
                .is_empty()
        );
        // On: it runs with the new token.
        admin
            .post_json(&url("/second/enable"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        let secret = secret_of(&fake, "rdp-cloudflared-second");
        assert_eq!(
            fake.secrets.lock().unwrap()[&secret].1,
            OTHER_TOKEN.as_bytes()
        );
        // A tunnel that is on is run again with the token it gets.
        replace(TOKEN).await.assert_status(StatusCode::OK);
        let secret = secret_of(&fake, "rdp-cloudflared-second");
        assert_eq!(fake.secrets.lock().unwrap()[&secret].1, TOKEN.as_bytes());
        assert_eq!(
            tunnel_service::running_tokens(app.state()).await.unwrap(),
            vec![("second".to_owned(), TOKEN.to_owned())]
        );
        // Unknown: not found, never made.
        admin
            .send(
                Method::PATCH,
                &url("/missing"),
                Some(&json!({"token": OTHER_TOKEN})),
            )
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
);
