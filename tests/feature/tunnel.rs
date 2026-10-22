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
