//! A project's members: a user of the panel without roles sees and acts on
//! the projects they are a member of, as their role there allows — never
//! another project's records, whichever route or id they try.

use rdockpanel::database::migrations::Migrator;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestDb, db_test};
use serde_json::{Value, json};

use crate::deployments::{action, create, records};
use crate::support::{self, fake_docker::FakeDocker};

db_test!(
    members_see_and_act_on_their_projects_only,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let shop = create(&app, &admin, "projects", json!({"name": "shop"})).await;
        let other = create(&app, &admin, "projects", json!({"name": "other"})).await;
        let web: i64 = create(
            &app,
            &admin,
            "services",
            json!({"project_id": shop, "name": "web", "image": "nginx:1.27", "replicas": 1}),
        )
        .await
        .parse()
        .unwrap();
        let api: i64 = create(
            &app,
            &admin,
            "services",
            json!({"project_id": other, "name": "api", "image": "nginx:1.27", "replicas": 1}),
        )
        .await
        .parse()
        .unwrap();
        create(&app, &admin, "service_domains", json!({"service_id": web.to_string(), "host": "shop.localhost", "path": "/", "port": 80, "https": false})).await;
        create(&app, &admin, "service_domains", json!({"service_id": api.to_string(), "host": "api.localhost", "path": "/", "port": 80, "https": false})).await;
        let x = |p: &str| app.admin_api(&format!("x{p}"));
        let members = x(&format!("/apps/projects/{shop}/members"));

        let user = support::user_with(&app, "dev", &[]).await;
        let dev = app.acting_as(&user).await;
        // Nobody's member yet: nothing, and no page.
        assert_eq!(
            dev.get(&x("/apps/overview")).await.json()["projects"],
            json!([])
        );
        dev.get(&x(&format!("/apps/services/{web}")))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // A developer of shop.
        admin
            .post_json(
                &members,
                &json!({"email": "nobody@example.test", "role": "developer"}),
            )
            .await
            .assert_validation_error("email");
        admin
            .post_json(
                &members,
                &json!({"email": "dev@example.test", "role": "owner"}),
            )
            .await
            .assert_validation_error("role");
        let added = admin
            .post_json(
                &members,
                &json!({"email": "dev@example.test", "role": "developer"}),
            )
            .await;
        added.assert_status(StatusCode::OK);
        assert_eq!(added.json()["members"][0]["role"], "developer");
        let dev = app.acting_as(&user).await;
        let seen = dev.get(&x("/apps/overview")).await.json();
        let projects = seen["projects"].as_array().unwrap();
        assert_eq!(projects.len(), 1, "{seen}");
        assert_eq!(projects[0]["name"], "shop");
        assert_eq!(projects[0]["role"], "developer");
        assert_eq!(projects[0]["can"]["create_service"], true);
        assert_eq!(projects[0]["can"]["members"], false);
        assert_eq!(projects[0]["services"][0]["name"], "web");

        // Its service: seen, changed, its secrets; another project's: never.
        let page = dev.get(&x(&format!("/apps/services/{web}"))).await;
        page.assert_status(StatusCode::OK);
        assert_eq!(page.json()["can"]["update"], true);
        assert_eq!(page.json()["can"]["destroy"], false);
        assert_eq!(page.json()["can"]["exec"], true);
        dev.get(&x(&format!("/apps/services/{web}/env")))
            .await
            .assert_status(StatusCode::OK);
        for path in [
            format!("/apps/services/{api}"),
            format!("/apps/services/{api}/env"),
            format!("/apps/services/{api}/errors"),
            format!("/apps/services/{api}/backups"),
        ] {
            dev.get(&x(&path))
                .await
                .assert_status(StatusCode::FORBIDDEN);
        }
        dev.send(
            Method::PUT,
            &x(&format!("/apps/services/{api}/env")),
            Some(&json!({"text": "A=1"})),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
        dev.post_json(
            &x(&format!("/apps/projects/{other}/services")),
            &json!({"name": "x", "kind": "app", "image": "nginx"}),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);

        // The resources: their projects' records only.
        let listed = dev.get(&records(&app, "services")).await;
        listed.assert_status(StatusCode::OK);
        let keys: Vec<Value> = listed.json()["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["key"].clone())
            .collect();
        assert_eq!(keys, [json!(web.to_string())], "{}", listed.text());
        let domains = dev.get(&records(&app, "service_domains")).await.json();
        assert_eq!(domains["data"].as_array().unwrap().len(), 1, "{domains}");
        let one = dev
            .get(&format!("{}/{api}", records(&app, "services")))
            .await;
        assert!(matches!(one.status.as_u16(), 403 | 404), "{}", one.status);
        let changed = dev
            .send(
                Method::PATCH,
                &format!("{}/{web}", records(&app, "services")),
                Some(&json!({"replicas": 2})),
            )
            .await;
        assert!(changed.status.is_success(), "{}", changed.text());
        let theirs = dev
            .send(
                Method::PATCH,
                &format!("{}/{api}", records(&app, "services")),
                Some(&json!({"replicas": 2})),
            )
            .await;
        assert!(
            matches!(theirs.status.as_u16(), 403 | 404),
            "{}",
            theirs.status
        );
        // Another project is not even one to choose (invalid, as a missing one).
        let sneak = dev
            .post_json(
                &records(&app, "services"),
                &json!({"project_id": other, "name": "sneak", "image": "nginx", "replicas": 1}),
            )
            .await;
        assert!(
            matches!(sneak.status.as_u16(), 403 | 422),
            "{}",
            sneak.text()
        );
        // Deploys, but never destroys (an admin's).
        action(&app, &dev, "services", "deploy", &web.to_string())
            .await
            .assert_status(StatusCode::OK);
        let destroyed = action(&app, &dev, "services", "destroy", &web.to_string()).await;
        assert!(
            !destroyed.status.is_success() || destroyed.json()["denied"] != json!([]),
            "{}",
            destroyed.text()
        );
        // The Domains page: theirs.
        let page = dev.get(&x("/apps/domains")).await.json();
        let hosts: Vec<Value> = page["domains"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["host"].clone())
            .collect();
        assert_eq!(hosts, [json!("shop.localhost")], "{page}");
        // Members are an admin's.
        dev.get(&members).await.assert_status(StatusCode::FORBIDDEN);

        // A viewer of other too: sees it, creates nothing there (whatever
        // they may in shop).
        admin
            .post_json(
                &x(&format!("/apps/projects/{other}/members")),
                &json!({"email": "dev@example.test", "role": "viewer"}),
            )
            .await
            .assert_status(StatusCode::OK);
        let dev = app.acting_as(&user).await;
        dev.get(&x(&format!("/apps/services/{api}")))
            .await
            .assert_status(StatusCode::OK);
        let domain = dev
            .post_json(
                &records(&app, "service_domains"),
                &json!({"service_id": api.to_string(), "host": "sneak.localhost", "path": "/", "port": 80, "https": false}),
            )
            .await;
        assert_eq!(domain.status, StatusCode::FORBIDDEN, "{}", domain.text());
        let service = dev
            .post_json(
                &records(&app, "services"),
                &json!({"project_id": other, "name": "sneak", "image": "nginx", "replicas": 1}),
            )
            .await;
        assert_eq!(service.status, StatusCode::FORBIDDEN, "{}", service.text());
        admin
            .send(
                Method::DELETE,
                &x(&format!("/apps/projects/{other}/members/{}", user.id)),
                None,
            )
            .await
            .assert_status(StatusCode::OK);

        // A viewer: sees, changes nothing.
        admin
            .send(
                Method::PATCH,
                &format!("{members}/{}", user.id),
                Some(&json!({"role": "viewer"})),
            )
            .await
            .assert_status(StatusCode::OK);
        let viewer = app.acting_as(&user).await;
        viewer
            .get(&x(&format!("/apps/services/{web}")))
            .await
            .assert_status(StatusCode::OK);
        viewer
            .get(&x(&format!("/apps/services/{web}/env")))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        let patched = viewer
            .send(
                Method::PATCH,
                &format!("{}/{web}", records(&app, "services")),
                Some(&json!({"replicas": 1})),
            )
            .await;
        assert_eq!(patched.status, StatusCode::FORBIDDEN, "{}", patched.text());

        // An admin manages members, but keeps one admin.
        admin
            .send(
                Method::PATCH,
                &format!("{members}/{}", user.id),
                Some(&json!({"role": "admin"})),
            )
            .await
            .assert_status(StatusCode::OK);
        let owner = app.acting_as(&user).await;
        owner.get(&members).await.assert_status(StatusCode::OK);
        owner
            .send(
                Method::PATCH,
                &format!("{members}/{}", user.id),
                Some(&json!({"role": "viewer"})),
            )
            .await
            .assert_validation_error("role");
        owner
            .send(Method::DELETE, &format!("{members}/{}", user.id), None)
            .await
            .assert_validation_error("role");
        // Never another project's members.
        owner
            .get(&x(&format!("/apps/projects/{other}/members")))
            .await
            .assert_status(StatusCode::FORBIDDEN);

        // Removed: nothing again (a role's grant of the pages taken back too).
        admin
            .send(Method::DELETE, &format!("{members}/{}", user.id), None)
            .await
            .assert_status(StatusCode::OK);
        let gone = app.acting_as(&user).await;
        assert_eq!(
            gone.get(&x("/apps/overview")).await.json()["projects"],
            json!([])
        );
        gone.get(&x(&format!("/apps/services/{web}")))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        assert_eq!(
            gone.get(&records(&app, "services")).await.status,
            StatusCode::FORBIDDEN
        );
    }
);

db_test!(
    the_roles_page_shows_areas_labels_and_presets,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let permissions = admin.get(&app.admin_api("system/permissions")).await;
        permissions.assert_status(StatusCode::OK);
        let list = permissions.json();
        let find = |name: &str| {
            list.as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .cloned()
                .unwrap_or_else(|| panic!("{name} in {list}"))
        };
        let deploy = find("services.deploy");
        assert_eq!(deploy["label"], "Deploy services");
        assert_eq!(deploy["area"], "apps");
        assert_eq!(find("docker.manage")["area"], "docker");
        assert_eq!(find("system.manage")["area"], "system");
        // A project role's permissions, as a starting selection for a role.
        let presets = admin
            .get(&app.admin_api("system/permission-presets"))
            .await
            .json();
        let ids: Vec<&str> = presets
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            ["viewer", "developer", "admin", "monitor"],
            "{presets}"
        );
        let viewer = presets[0]["permissions"].to_string();
        assert!(viewer.contains("apps.access") && viewer.contains("services.view"));
        assert!(!viewer.contains("services.deploy") && !viewer.contains("system."));
    }
);
