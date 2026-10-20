//! Certificate resolvers: written through their resource by the panel's
//! administrators only (the DNS token sealed, never shown), applied to
//! Traefik (its own arguments, the token as a Swarm secret, the installer's
//! arguments kept), given again after an installer run, chosen per domain
//! (a wildcard over DNS, one resolver per host) and used by its routes.

use rdockpanel::database::migrations::Migrator;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestApp, TestClient, TestDb, db_test};
use serde_json::{Value, json};

use crate::deployments::{create, records};
use crate::support::{self, fake_docker::FakeDocker};

/// The installer's Traefik: its resolver among its arguments.
fn installer_traefik(fake: &FakeDocker) {
    let mut services = fake.services.lock().unwrap();
    let spec = services
        .get_mut("rdp-traefik")
        .unwrap()
        .spec
        .as_mut()
        .unwrap();
    let container = spec
        .task_template
        .as_mut()
        .unwrap()
        .container_spec
        .as_mut()
        .unwrap();
    container.args = Some(vec![
        "--providers.file.directory=/etc/traefik/dynamic".to_owned(),
        "--certificatesresolvers.letsencrypt.acme.email=ops@example.com".to_owned(),
        "--certificatesresolvers.letsencrypt.acme.httpchallenge.entrypoint=web".to_owned(),
        // An administrator's own, added by hand: never the panel's.
        "--certificatesresolvers.mine.acme.email=me@example.com".to_owned(),
    ]);
}

fn traefik_spec(fake: &FakeDocker) -> Value {
    let services = fake.services.lock().unwrap();
    serde_json::to_value(services["rdp-traefik"].spec.as_ref().unwrap()).unwrap()
}

fn args(fake: &FakeDocker) -> Vec<String> {
    serde_json::from_value(traefik_spec(fake)["TaskTemplate"]["ContainerSpec"]["Args"].clone())
        .unwrap()
}

async fn admin(app: &TestApp) -> TestClient<'_> {
    let user = support::user_with(app, "admin", &[]).await;
    app.make_super_admin(&user).await;
    app.acting_as(&user).await
}

fn errors(response: &rustcms_testing::TestResponse) -> String {
    response.json()["errors"].to_string()
}

db_test!(
    resolvers_are_checked_and_administrators_only,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        let url = records(&app, "cert_resolvers");
        for (body, field) in [
            (
                json!({"name": "letsencrypt", "kind": "http", "email": "ops@example.com"}),
                "name",
            ),
            (
                json!({"name": "Bad-Name", "kind": "http", "email": "ops@example.com"}),
                "name",
            ),
            (
                json!({"name": "le", "kind": "http", "email": "nope"}),
                "email",
            ),
            (
                json!({"name": "cf", "kind": "dns", "email": "ops@example.com", "token": {"replace": "t"}}),
                "provider",
            ),
            (
                json!({"name": "cf", "kind": "dns", "provider": "cloudflare", "email": "ops@example.com"}),
                "token",
            ),
            (
                json!({"name": "cf", "kind": "dns", "provider": "nowhere", "email": "ops@example.com", "token": {"replace": "t"}}),
                "provider",
            ),
        ] {
            let response = admin.post_json(&url, &body).await;
            response.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
            assert!(
                errors(&response).contains(field),
                "{body}: {}",
                errors(&response)
            );
        }
        let cf = create(&app, &admin, "cert_resolvers", json!({"name": "cf", "kind": "dns", "provider": "cloudflare", "email": "ops@example.com", "token": {"replace": "secret-token"}})).await;
        // One resolver per provider (its token is one variable of Traefik's).
        let again = admin
            .post_json(&url, &json!({"name": "cf2", "kind": "dns", "provider": "cloudflare", "email": "ops@example.com", "token": {"replace": "x"}}))
            .await;
        again.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        // The token never comes back.
        let shown = admin.get(&format!("{url}/{cf}")).await;
        shown.assert_status(StatusCode::OK);
        assert!(!shown.text().contains("secret-token"), "{}", shown.text());
        // A DNS resolver keeps a token: clearing it is refused.
        admin
            .send(
                Method::PATCH,
                &format!("{url}/{cf}"),
                Some(&json!({"token": {"clear": true}})),
            )
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        // Values stored as checked: trimmed.
        let spaced: i64 = create(&app, &admin, "cert_resolvers", json!({"name": " spaced ", "kind": "dns ", "provider": " digitalocean", "email": " ops@example.com ", "token": {"replace": "t"}})).await.parse().unwrap();
        {
            use sea_orm::EntityTrait;
            let row = rdockpanel::models::cert_resolver::Entity::find_by_id(spaced)
                .one(app.state().db())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                (
                    row.name.as_str(),
                    row.kind.as_str(),
                    row.provider.as_deref(),
                    row.email.as_str()
                ),
                ("spaced", "dns", Some("digitalocean"), "ops@example.com")
            );
        }
        // An HTTP resolver needs none.
        create(
            &app,
            &admin,
            "cert_resolvers",
            json!({"name": "le", "kind": "http", "email": "ops@example.com"}),
        )
        .await;
        // Its name never changes (routes refer to it).
        let renamed = admin
            .send(
                Method::PATCH,
                &format!("{url}/{cf}"),
                Some(&json!({"name": "other"})),
            )
            .await;
        renamed.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        // Not for a user managing services and domains.
        let operator = support::user_with(
            &app,
            "operator",
            &["services.view_any", "service_domains.create"],
        )
        .await;
        let operator = app.acting_as(&operator).await;
        operator
            .get(&url)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        operator
            .post_json(
                &url,
                &json!({"name": "le", "kind": "http", "email": "ops@example.com"}),
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        operator
            .get(&app.admin_api("x/apps/traefik/resolvers"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }
);

db_test!(
    applying_gives_traefik_its_resolvers_and_keeps_the_installers,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        installer_traefik(&fake);
        let admin = admin(&app).await;
        let status = app.admin_api("x/apps/traefik/resolvers");
        let apply = app.admin_api("x/apps/traefik/apply");
        // Nothing of the panel's yet: applied.
        assert_eq!(admin.get(&status).await.json()["applied"], true);
        let cf = create(&app, &admin, "cert_resolvers", json!({"name": "cf", "kind": "dns", "provider": "cloudflare", "email": "ops@example.com", "token": {"replace": "secret-token"}})).await;
        create(
            &app,
            &admin,
            "cert_resolvers",
            json!({"name": "le2", "kind": "http", "email": "ops@example.com", "staging": true}),
        )
        .await;
        assert_eq!(admin.get(&status).await.json()["applied"], false);

        admin
            .post_json(&apply, &json!({}))
            .await
            .assert_status(StatusCode::OK);
        let applied = args(&fake);
        for arg in [
            "--providers.file.directory=/etc/traefik/dynamic",
            "--certificatesresolvers.letsencrypt.acme.email=ops@example.com",
            "--certificatesresolvers.cf.acme.dnschallenge.provider=cloudflare",
            "--certificatesresolvers.cf.acme.storage=/etc/traefik/acme/cf.json",
            "--certificatesresolvers.le2.acme.httpchallenge.entrypoint=web",
            "--certificatesresolvers.le2.acme.caserver=https://acme-staging-v02.api.letsencrypt.org/directory",
        ] {
            assert!(
                applied.iter().any(|a| a == arg),
                "{arg} missing: {applied:?}"
            );
        }
        // The token: a Swarm secret, read through `_FILE`; never in the spec.
        let spec = traefik_spec(&fake);
        assert!(!spec.to_string().contains("secret-token"), "{spec}");
        let secrets = fake.secrets.lock().unwrap().clone();
        assert_eq!(secrets.len(), 1);
        let (name, (_, content, _)) = secrets.iter().next().unwrap();
        assert_eq!(content, b"secret-token");
        let env = spec["TaskTemplate"]["ContainerSpec"]["Env"].to_string();
        assert!(
            env.contains(&format!("CF_DNS_API_TOKEN_FILE=/run/secrets/{name}")),
            "{env}"
        );
        assert_eq!(
            spec["TaskTemplate"]["Networks"][0]["Target"],
            "ingress-extra"
        );
        assert_eq!(admin.get(&status).await.json()["applied"], true);

        // Applied again with nothing new: Traefik left alone.
        let updates = fake
            .calls()
            .iter()
            .filter(|c| *c == "update service rdp-traefik")
            .count();
        admin
            .post_json(&apply, &json!({}))
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(
            fake.calls()
                .iter()
                .filter(|c| *c == "update service rdp-traefik")
                .count(),
            updates
        );

        // An installer run replaces the arguments: the panel gives its own again.
        installer_traefik(&fake);
        let plan = {
            let state = app.state();
            let resolvers =
                rdockpanel::services::cert_service::resolvers(state.db(), state.keyring())
                    .await
                    .unwrap();
            rdockpanel::services::cert_service::plan(
                &resolvers,
                &rdockpanel::services::cert_service::plan_key(state.keyring()),
            )
        };
        let docker = app
            .state()
            .service::<rdockpanel::services::docker_service::Docker>()
            .unwrap();
        let traefik = app
            .state()
            .service::<rdockpanel::services::traefik_service::Traefik>()
            .unwrap();
        assert!(
            rdockpanel::services::cert_service::reconcile(&docker, &traefik, &plan)
                .await
                .unwrap()
        );
        assert!(
            args(&fake)
                .iter()
                .any(|a| a == "--certificatesresolvers.cf.acme.dnschallenge.provider=cloudflare")
        );
        assert!(
            !rdockpanel::services::cert_service::reconcile(&docker, &traefik, &plan)
                .await
                .unwrap()
        );
        assert!(
            args(&fake).iter().any(|a| a.contains(".mine.")),
            "an administrator's own resolver stays"
        );
        // Its token's variable lost: given again too.
        {
            let mut services = fake.services.lock().unwrap();
            let spec = services
                .get_mut("rdp-traefik")
                .unwrap()
                .spec
                .as_mut()
                .unwrap();
            let container = spec
                .task_template
                .as_mut()
                .unwrap()
                .container_spec
                .as_mut()
                .unwrap();
            // Lost, and another value of the same variable (a wrong path).
            container.env = Some(vec!["CF_DNS_API_TOKEN_FILE=/wrong/path".to_owned()]);
        }
        assert!(
            rdockpanel::services::cert_service::reconcile(&docker, &traefik, &plan)
                .await
                .unwrap()
        );
        let env: Vec<String> = serde_json::from_value(
            traefik_spec(&fake)["TaskTemplate"]["ContainerSpec"]["Env"].clone(),
        )
        .unwrap();
        assert_eq!(env.len(), 1, "{env:?}");
        assert!(
            env[0].starts_with("CF_DNS_API_TOKEN_FILE=/run/secrets/rdp-dns-cf-"),
            "{env:?}"
        );

        // Deleted and applied: its arguments and secret go, the installer's stay.
        admin
            .send(
                Method::DELETE,
                &format!("{}/{cf}", records(&app, "cert_resolvers")),
                None,
            )
            .await
            .assert_status(StatusCode::NO_CONTENT);
        admin
            .post_json(&apply, &json!({}))
            .await
            .assert_status(StatusCode::OK);
        let left = args(&fake);
        assert!(!left.iter().any(|a| a.contains(".cf.")), "{left:?}");
        assert!(left.iter().any(|a| a.contains(".letsencrypt.")), "{left:?}");
        assert!(left.iter().any(|a| a.contains(".mine.")), "{left:?}");
        assert!(fake.secrets.lock().unwrap().is_empty());
        assert!(
            !traefik_spec(&fake)["TaskTemplate"]["ContainerSpec"]["Env"]
                .to_string()
                .contains("CF_DNS")
        );
    }
);

db_test!(
    domains_choose_their_resolver,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        let cf: i64 = create(&app, &admin, "cert_resolvers", json!({"name": "cf", "kind": "dns", "provider": "cloudflare", "email": "ops@example.com", "token": {"replace": "t"}})).await.parse().unwrap();
        let http: i64 = create(
            &app,
            &admin,
            "cert_resolvers",
            json!({"name": "web", "kind": "http", "email": "ops@example.com"}),
        )
        .await
        .parse()
        .unwrap();
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        let url = records(&app, "service_domains");
        let domain = |host: &str, resolver: Option<i64>| json!({"service_id": service, "host": host, "path": "/", "port": 80, "https": true, "cert_resolver_id": resolver.map(|r| r.to_string())});
        // A wildcard's certificate over HTTP: refused; over DNS: fine.
        let refused = admin
            .post_json(&url, &domain("*.shop.example.com", Some(http)))
            .await;
        refused.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(errors(&refused).contains("DNS"), "{}", errors(&refused));
        // HTTPS left out: on (the column's default), so checked too.
        admin
            .post_json(&url, &json!({"service_id": service, "host": "*.other.example.com", "path": "/", "port": 80, "cert_resolver_id": http.to_string()}))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        create(
            &app,
            &admin,
            "service_domains",
            domain("*.shop.example.com", Some(cf)),
        )
        .await;
        // The wildcard keeps its resolver on DNS.
        admin
            .send(
                Method::PATCH,
                &format!("{}/{cf}", records(&app, "cert_resolvers")),
                Some(&json!({"kind": "http", "provider": null, "token": {"clear": true}})),
            )
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        // One host, one certificate: its other paths choose the same.
        let path = |resolver: Option<i64>| json!({"service_id": service, "host": "shop.example.com", "path": "/api", "port": 80, "https": true, "cert_resolver_id": resolver.map(|r| r.to_string())});
        create(
            &app,
            &admin,
            "service_domains",
            domain("shop.example.com", Some(http)),
        )
        .await;
        admin
            .post_json(&url, &path(Some(cf)))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        admin
            .post_json(&url, &path(None))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        create(&app, &admin, "service_domains", path(Some(http))).await;
        // Resolvers in use cannot be deleted.
        admin
            .send(
                Method::DELETE,
                &format!("{}/{http}", records(&app, "cert_resolvers")),
                None,
            )
            .await
            .assert_status(StatusCode::FORBIDDEN);
        // A domain manager may not choose one, nor change a domain that has one.
        let operator = support::user_with(
            &app,
            "operator",
            &[
                "services.view_any",
                "services.view",
                "service_domains.view_any",
                "service_domains.view",
                "service_domains.create",
                "service_domains.update",
                "service_domains.delete",
            ],
        )
        .await;
        let operator = app.acting_as(&operator).await;
        operator
            .post_json(&url, &domain("api.example.com", Some(http)))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        create(&app, &operator, "service_domains", json!({"service_id": service, "host": "plain.example.com", "path": "/", "port": 80, "https": true})).await;

        // Its routes ask its resolver.
        let service_id: i64 = service.parse().unwrap();
        let snapshot =
            rdockpanel::services::deployment_service::snapshot_without_env(app.state().db(), &{
                use sea_orm::EntityTrait;
                rdockpanel::models::service::Entity::find_by_id(service_id)
                    .one(app.state().db())
                    .await
                    .unwrap()
                    .unwrap()
            })
            .await
            .unwrap();
        let routes = rdockpanel::services::deploy_service::routes(
            &snapshot,
            &rdockpanel::services::deploy_service::DeployOptions::default(),
        )
        .unwrap();
        let yaml = routes.to_yaml();
        assert!(
            routes.uses_resolver("cf") && routes.uses_resolver("web"),
            "{yaml}"
        );
        assert!(yaml.contains("certResolver: cf"), "{yaml}");
    }
);

/// A certificate for `names` signed by a new authority: (chain, key).
fn issued(names: &[&str]) -> (String, String) {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    let issuer = Issuer::new(ca, ca_key);
    let key = KeyPair::generate().unwrap();
    let params =
        CertificateParams::new(names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>()).unwrap();
    let cert = params.signed_by(&key, &issuer).unwrap();
    (
        format!("{}{}", cert.pem(), ca_cert.pem()),
        key.serialize_pem(),
    )
}

db_test!(
    uploaded_certificates_are_checked_served_and_chosen,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        let admin = admin(&app).await;
        let url = app.admin_api("x/apps/certificates");
        let (chain, key) = issued(&["shop.example.com", "*.shop.example.com"]);
        let (_, other_key) = issued(&["shop.example.com"]);
        // Another key: refused, on the key.
        let refused = admin
            .post_json(&url, &json!({"chain": chain, "key": other_key}))
            .await;
        refused.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(errors(&refused).contains("key"), "{}", errors(&refused));
        let made = admin
            .post_json(&url, &json!({"name": "shop", "chain": chain, "key": key}))
            .await;
        made.assert_status(StatusCode::OK);
        let id = made.json()["id"].as_i64().unwrap();
        // The same one again: refused.
        admin
            .post_json(&url, &json!({"chain": chain, "key": key}))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        // Listed without its key; written for Traefik, the key private.
        let listed = admin.get(&url).await;
        assert!(!listed.text().contains("PRIVATE KEY"), "{}", listed.text());
        assert_eq!(
            listed.json()["certificates"][0]["names"][1],
            "*.shop.example.com"
        );
        let dir = support::traefik_dir(&app);
        let store = std::fs::read_to_string(dir.join("rdp-certificates.yml")).unwrap();
        assert!(
            store.contains(&format!("/etc/traefik/dynamic/certs/{id}-")),
            "{store}"
        );
        let key_file = std::fs::read_dir(dir.join("certs"))
            .unwrap()
            .flatten()
            .find(|e| e.file_name().to_string_lossy().ends_with(".key"))
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                key_file.metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
        }

        // Chosen by a domain it covers: a wildcard too, with no DNS resolver.
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        let domains = records(&app, "service_domains");
        let domain = |host: &str| json!({"service_id": service, "host": host, "path": "/", "port": 80, "https": true, "certificate_id": id.to_string()});
        let uncovered = admin
            .post_json(&domains, &domain("other.example.com"))
            .await;
        uncovered.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            errors(&uncovered).contains("cover"),
            "{}",
            errors(&uncovered)
        );
        create(
            &app,
            &admin,
            "service_domains",
            domain("*.shop.example.com"),
        )
        .await;
        create(&app, &admin, "service_domains", domain("shop.example.com")).await;
        // Not with a resolver too.
        let resolver = create(
            &app,
            &admin,
            "cert_resolvers",
            json!({"name": "le", "kind": "http", "email": "ops@example.com"}),
        )
        .await;
        let both = json!({"service_id": service, "host": "a.shop.example.com", "path": "/", "port": 80, "https": true, "certificate_id": id.to_string(), "cert_resolver_id": resolver});
        admin
            .post_json(&domains, &both)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

        // Its routes: TLS with no resolver (Traefik picks it by name).
        let service_id: i64 = service.parse().unwrap();
        let row = {
            use sea_orm::EntityTrait;
            rdockpanel::models::service::Entity::find_by_id(service_id)
                .one(app.state().db())
                .await
                .unwrap()
                .unwrap()
        };
        let snapshot =
            rdockpanel::services::deployment_service::snapshot_without_env(app.state().db(), &row)
                .await
                .unwrap();
        let options = rdockpanel::services::deploy_service::DeployOptions {
            cert_resolver: "letsencrypt".to_owned(),
            ..Default::default()
        };
        let yaml = rdockpanel::services::deploy_service::routes(&snapshot, &options)
            .unwrap()
            .to_yaml();
        assert!(!yaml.contains("certResolver"), "{yaml}");

        // An HTTP-only domain with it: no part of what it must cover.
        create(
            &app,
            &admin,
            "service_domains",
            json!({"service_id": service, "host": "plain.example.org", "path": "/", "port": 80, "https": false, "certificate_id": id.to_string()}),
        )
        .await;
        // In use: not deleted; replaced only by one covering its domains.
        admin
            .send(Method::DELETE, &format!("{url}/{id}"), None)
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let (narrow, narrow_key) = issued(&["shop.example.com"]);
        admin
            .send(
                Method::PUT,
                &format!("{url}/{id}"),
                Some(&json!({"chain": narrow, "key": narrow_key})),
            )
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let (renewed, renewed_key) = issued(&["shop.example.com", "*.shop.example.com"]);
        admin
            .send(
                Method::PUT,
                &format!("{url}/{id}"),
                Some(&json!({"chain": renewed, "key": renewed_key})),
            )
            .await
            .assert_status(StatusCode::OK);
        let renewed_store = std::fs::read_to_string(dir.join("rdp-certificates.yml")).unwrap();
        assert_ne!(store, renewed_store);

        // A deployment asked with it, the certificate gone since: it fails.
        let spec = serde_json::to_value(&snapshot).unwrap();
        assert!(
            rdockpanel::services::certificate_service::check_snapshot(app.state().db(), &spec)
                .await
                .is_ok()
        );
        let mut gone = spec.clone();
        for domain in gone["domains"].as_array_mut().unwrap() {
            if domain.get("certificate").is_some() {
                domain["certificate"] = json!("999999");
            }
        }
        let why =
            rdockpanel::services::certificate_service::check_snapshot(app.state().db(), &gone)
                .await
                .unwrap_err();
        assert!(why.contains("deleted"), "{why}");

        // Not for a user managing domains.
        let operator = support::user_with(
            &app,
            "operator",
            &["services.view_any", "service_domains.create"],
        )
        .await;
        let operator = app.acting_as(&operator).await;
        operator
            .get(&url)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        operator
            .post_json(&domains, &domain("b.shop.example.com"))
            .await
            .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    }
);

db_test!(
    the_traefik_page_shows_restarts_and_keeps_middlewares,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        installer_traefik(&fake);
        let admin = admin(&app).await;
        // Its state, never its arguments.
        let status = admin.get(&app.admin_api("x/apps/traefik/status")).await;
        status.assert_status(StatusCode::OK);
        assert_eq!(status.json()["traefik"]["image"], "traefik:v3.7");
        assert!(
            !status.text().contains("certificatesresolvers"),
            "{}",
            status.text()
        );

        // A restart: its tasks replaced, logged.
        let before = fake
            .calls()
            .iter()
            .filter(|c| *c == "update service rdp-traefik")
            .count();
        admin
            .post_json(&app.admin_api("x/apps/traefik/restart"), &json!({}))
            .await
            .assert_status(StatusCode::OK);
        assert_eq!(
            fake.calls()
                .iter()
                .filter(|c| *c == "update service rdp-traefik")
                .count(),
            before + 1
        );
        assert!(
            args(&fake).iter().any(|a| a.contains(".mine.")),
            "the rest kept"
        );

        // Middlewares: checked, kept, written for Traefik.
        let url = app.admin_api("x/apps/traefik/middlewares");
        let refused = admin
            .send(
                Method::PUT,
                &url,
                Some(&json!({"yaml": "custom-x:\n  forwardAuth:\n    address: http://evil\n"})),
            )
            .await;
        refused.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            errors(&refused).contains("not allowed"),
            "{}",
            errors(&refused)
        );
        let yaml = "custom-limit:\n  rateLimit:\n    average: 50\n";
        let saved = admin
            .send(Method::PUT, &url, Some(&json!({"yaml": yaml})))
            .await;
        saved.assert_status(StatusCode::OK);
        assert_eq!(saved.json()["middlewares"][0], "custom-limit@file");
        let file =
            std::fs::read_to_string(support::traefik_dir(&app).join("rdp-custom-middlewares.yml"))
                .unwrap();
        assert!(
            file.contains("custom-limit") && file.contains("average: 50"),
            "{file}"
        );
        assert_eq!(
            admin
                .get(&app.admin_api("x/apps/traefik/status"))
                .await
                .json()["middlewares"],
            yaml
        );
        // A domain uses one.
        let project = create(&app, &admin, "projects", json!({"name": "mw"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        create(&app, &admin, "service_domains", json!({"service_id": service, "host": "mw.example.com", "path": "/", "port": 80, "https": false, "middlewares": "custom-limit@file"})).await;

        // Not for others.
        let operator = support::user_with(
            &app,
            "operator",
            &["docker.view", "docker.manage", "docker.logs"],
        )
        .await;
        let operator = app.acting_as(&operator).await;
        operator
            .get(&app.admin_api("x/apps/traefik/status"))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        operator
            .send(Method::PUT, &url, Some(&json!({"yaml": ""})))
            .await
            .assert_status(StatusCode::FORBIDDEN);
    }
);

db_test!(
    the_panels_domain_is_checked_and_given_as_a_command,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::default();
        let app = support::app_with_docker(db, fake.clone()).await;
        installer_traefik(&fake);
        let admin = admin(&app).await;
        // As the installer set it: its email from Traefik's arguments.
        let current = admin.get(&app.admin_api("x/apps/panel")).await;
        current.assert_status(StatusCode::OK);
        assert_eq!(current.json()["email"], "ops@example.com");
        // Where it runs (APP_URL), not the installer's default port.
        assert!(
            current.json()["url"]
                .as_str()
                .is_some_and(|u| u.starts_with("http")),
            "{}",
            current.text()
        );
        assert_eq!(current.json()["installed"], false);

        let url = app.admin_api("x/apps/panel/check");
        let bad = admin
            .post_json(&url, &json!({"domain": "Not A Host", "email": "nope", "apps_domain": "*.apps.example.com"}))
            .await
            .json();
        assert!(
            bad["errors"]["domain"].is_string()
                && bad["errors"]["email"].is_string()
                && bad["errors"]["apps_domain"].is_string(),
            "{bad}"
        );
        assert!(bad["command"].is_null());
        // A service's domain is not the panel's.
        let project = create(&app, &admin, "projects", json!({"name": "taken"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        create(&app, &admin, "service_domains", json!({"service_id": service, "host": "*.example.invalid", "path": "/", "port": 80, "https": false})).await;
        let taken = admin
            .post_json(
                &url,
                &json!({"domain": "panel.example.invalid", "email": "ops@example.com"}),
            )
            .await
            .json();
        assert!(
            taken["errors"]["domain"]
                .as_str()
                .unwrap()
                .contains("service"),
            "{taken}"
        );
        // Fine: the command applying it (DNS said, not refused).
        let ok = admin
            .post_json(&url, &json!({"domain": "panel.invalid", "email": "ops@example.com", "apps_domain": "apps.invalid"}))
            .await
            .json();
        assert_eq!(ok["errors"], json!({}), "{ok}");
        let command = ok["command"].as_str().unwrap();
        assert!(
            command.ends_with(
                "--domain panel.invalid --email ops@example.com --apps-domain apps.invalid"
            ),
            "{command}"
        );
        assert!(
            ok["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w.as_str().unwrap().contains("does not resolve")),
            "{ok}"
        );
    }
);
