//! The Domains page (`/admin/api/x/apps/domains`): every service's domains,
//! where they lead and whether Traefik serves them as configured.

use rdockpanel::database::migrations::Migrator;
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestDb, db_test};
use serde_json::{Value, json};

use crate::deployments::{create, publish_routes, records, run};
use crate::support::{self, fake_docker::FakeDocker};

fn find<'a>(list: &'a Value, host: &str) -> &'a Value {
    list["domains"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["host"] == host)
        .unwrap_or_else(|| panic!("{host} in {list}"))
}

db_test!(
    every_domain_and_whether_it_is_served,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let user = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&user).await;
        let admin = app.acting_as(&user).await;
        let project = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let service = create(
            &app,
            &admin,
            "services",
            json!({"project_id": project, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await;
        let first = create(&app, &admin, "service_domains", json!({"service_id": service, "host": "shop.localhost", "path": "/", "port": 8080, "https": false})).await;
        let url = app.admin_api("x/apps/domains");

        // Never deployed: not served.
        let list = admin.get(&url).await;
        list.assert_status(StatusCode::OK);
        let list = list.json();
        let shop = find(&list, "shop.localhost");
        assert_eq!(shop["status"], "stopped", "{list}");
        assert_eq!(shop["target"], "http://shop_web:8080/");
        assert_eq!(shop["project"], "shop");
        assert_eq!(shop["primary"], true);
        assert_eq!(list["services"][0]["name"], "web");
        assert_eq!(list["can"]["update"], true);

        let deployed = run(&app, &admin, "deploy", service.parse().unwrap()).await;
        assert_eq!(deployed.status, "succeeded", "{:?}", deployed.error);
        let list = admin.get(&url).await.json();
        assert_eq!(find(&list, "shop.localhost")["status"], "live", "{list}");

        // Added, changed, removed: served as saved, without a deployment.
        let id: i64 = service.parse().unwrap();
        create(&app, &admin, "service_domains", json!({"service_id": service, "host": "api.localhost", "path": "/", "port": 80, "https": false})).await;
        publish_routes(&app, id).await;
        let list = admin.get(&url).await.json();
        assert_eq!(find(&list, "api.localhost")["status"], "live");
        assert_eq!(find(&list, "shop.localhost")["status"], "live");
        let removed = admin
            .send(
                rustcms::axum::http::Method::DELETE,
                &format!("{}/{first}", records(&app, "service_domains")),
                None,
            )
            .await;
        assert!(removed.status.is_success(), "{}", removed.text());
        // The page asks for the routes after a delete (no hook follows it).
        admin
            .post_json(
                &app.admin_api(&format!("x/apps/services/{id}/routes")),
                &json!({}),
            )
            .await
            .assert_status(StatusCode::ACCEPTED);
        publish_routes(&app, id).await;
        let routes =
            std::fs::read_to_string(support::traefik_dir(&app).join("shop_web.yml")).unwrap();
        assert!(!routes.contains("Host(`shop.localhost`)"), "{routes}");
        assert!(routes.contains("Host(`api.localhost`)"), "{routes}");
        let list = admin.get(&url).await.json();
        assert_eq!(list["domains"].as_array().unwrap().len(), 1, "{list}");
        let api = find(&list, "api.localhost");
        assert_eq!(api["status"], "live");
        assert_eq!(api["primary"], true);
        assert_eq!(api["writable"], true);
        assert_eq!(list["services"][0]["writable"], true);
        assert_eq!(list["default_resolver"], Value::Null);

        // Changed (as the form sends it, nullable fields too).
        let domain = api["id"].as_i64().unwrap();
        let changed = admin
            .send(
                rustcms::axum::http::Method::PATCH,
                &format!("{}/{domain}", records(&app, "service_domains")),
                Some(&json!({"host": "api.localhost", "path": "/", "port": 8081, "https": false, "middlewares": null, "cert_resolver_id": null, "certificate_id": null})),
            )
            .await;
        assert!(changed.status.is_success(), "{}", changed.text());
        let list = admin.get(&url).await.json();
        assert_eq!(find(&list, "api.localhost")["status"], "live");
        assert_eq!(
            find(&list, "api.localhost")["target"],
            "http://shop_web:8081/"
        );

        // Stopped: not served.
        let stopped = run(&app, &admin, "stop", id).await;
        assert_eq!(stopped.status, "succeeded", "{:?}", stopped.error);
        assert_eq!(
            find(&admin.get(&url).await.json(), "api.localhost")["status"],
            "stopped"
        );

        // Without the services' names: refused; without system.manage: no
        // certificate to choose.
        let only = support::user_with(&app, "only", &["service_domains.view_any"]).await;
        app.acting_as(&only)
            .await
            .get(&url)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        let none = support::user_with(&app, "none", &["services.view_any"]).await;
        app.acting_as(&none)
            .await
            .get(&url)
            .await
            .assert_status(StatusCode::FORBIDDEN);
        let reader = support::user_with(
            &app,
            "reader",
            &["services.view_any", "service_domains.view_any"],
        )
        .await;
        let seen = app.acting_as(&reader).await.get(&url).await;
        seen.assert_status(StatusCode::OK);
        let seen = seen.json();
        assert_eq!(seen["can"]["create"], false);
        assert_eq!(seen["can"]["system"], false);
        assert_eq!(seen["panel"], Value::Null);
        assert_eq!(seen["cert_resolvers"], json!([]));
        assert_eq!(find(&seen, "api.localhost")["status"], "stopped");
    }
);
