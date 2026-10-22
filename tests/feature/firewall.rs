//! The Firewall page: the host read through the helper container, the
//! policy saved (checked), and each change a transaction: applied (the
//! table journaled and loaded by the host, its revert armed), then kept in
//! time or reverted; one at a time; settled after an interruption.

use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::{firewall_transaction, panel_setting};
use rdockpanel::services::action_service::Initiator;
use rdockpanel::services::docker_service::{Docker, SYSTEM_LABEL, SwarmService};
use rdockpanel::services::firewall_service;
use rdockpanel::services::traefik_service::Traefik;
use rustcms::audit::AuditContext;
use rustcms::axum::http::{Method, StatusCode};
use rustcms_testing::{TestDb, db_test};
use sea_orm::{EntityTrait, QueryOrder};
use serde_json::json;

use crate::support::{self, fake_docker::FakeDocker};

const IMAGE: &str = "ghcr.io/irisvn/rdockpanel:main";

/// The kernel's listing of a table (what the hash is taken of).
const TABLE: &str = r#"{"nftables":[{"metainfo":{"version":"1.0.2"}},{"table":{"family":"inet","name":"rdockpanel","handle":3}},{"rule":{"family":"inet","table":"rdockpanel","chain":"input","handle":9,"expr":[{"drop":null}]}}]}"#;

fn discovered(fw: &str) -> String {
    format!(
        "== ssh\n22\n== listen\n\
         tcp LISTEN 0 4096 0.0.0.0:22 0.0.0.0:* users:((\"sshd\",pid=1,fd=3))\n\
         tcp LISTEN 0 4096 0.0.0.0:443 0.0.0.0:* users:((\"docker-proxy\",pid=2,fd=8))\n\
         tcp LISTEN 0 4096 *:2377 *:* users:((\"dockerd\",pid=3,fd=19))\n\
         tcp LISTEN 0 4096 0.0.0.0:5432 0.0.0.0:* users:((\"postgres\",pid=4,fd=5))\n\
         == tools\nnft=yes\nsystemd=yes\nufw=inactive\nfirewalld=inactive\nnftables=disabled\nnodes=1\n\
         == drivers\nbridge\noverlay\n\
         == bridges\n3f2a1b4c5d6e7f8091a2|docker0||\n\
         == route\neth0\n\
         == fw\n{fw}== end\n"
    )
}

/// What `fw.sh apply` prints: its arm time, then the table loaded.
fn applied() -> String {
    format!(
        "armed={}\n== table\n{TABLE}\n",
        chrono::Utc::now().timestamp()
    )
}

fn installed(docker: &FakeDocker) {
    let spec = json!({
        "Name": "rdockpanel",
        "Labels": {SYSTEM_LABEL: "true"},
        "TaskTemplate": {"ContainerSpec": {"Image": format!("{IMAGE}@sha256:aaa")}},
    });
    docker.services.lock().unwrap().insert(
        "rdockpanel".to_owned(),
        SwarmService {
            spec: Some(serde_json::from_value(spec).unwrap()),
            ..Default::default()
        },
    );
}

fn system() -> Initiator {
    Initiator {
        user_id: None,
        audit: AuditContext::system(None),
    }
}

db_test!(
    the_firewall_page_and_its_changes,
    migrate = Migrator,
    |db: TestDb| async move {
        let docker = FakeDocker::default();
        installed(&docker);
        let outputs = docker.host_outputs.clone();
        let calls = docker.calls.clone();
        let app = support::app_with_docker(db, docker).await;
        let state = app.state();
        let engine = state.service::<Docker>().unwrap();
        let by = system();

        // From source: nothing on the host is read.
        let page = firewall_service::report(state, &engine, false)
            .await
            .unwrap();
        assert_eq!(page["available"], false);
        assert!(calls.lock().unwrap().iter().all(|c| !c.starts_with("host")));

        // The policy, administrators' only, checked.
        let url = |p: &str| app.admin_api(&format!("x/apps/firewall{p}"));
        let user = support::user_with(&app, "dev", &["projects.view_any"]).await;
        let dev = app.acting_as(&user).await;
        dev.send(Method::POST, &url("/openings"), Some(&json!({"ports": "5432", "protocol": "tcp", "target": "host", "sources_v4": "10.8.0.0/24"})))
            .await
            .assert_status(StatusCode::FORBIDDEN);
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let open = |body: serde_json::Value| {
            let admin = &admin;
            let url = url("/openings");
            async move { admin.send(Method::POST, &url, Some(&body)).await }
        };
        open(json!({"ports": "22", "protocol": "tcp", "target": "host", "sources_v4": "10.0.0.1"}))
            .await
            .assert_validation_error("ports");
        open(json!({"ports": "5432", "protocol": "tcp", "target": "published"}))
            .await
            .assert_validation_error("sources_v4");
        open(json!({"ports": "5432", "protocol": "tcp", "target": "published", "sources_v4": "2001:db8::/32"}))
            .await
            .assert_validation_error("sources_v4");
        open(json!({"ports": "5432", "protocol": "tcp", "target": "published", "sources_v4": "10.0.0.0/8; drop"}))
            .await
            .assert_validation_error("sources_v4");
        let opened = open(json!({"ports": "5432", "protocol": "tcp", "target": "published", "sources_v4": "10.8.0.7/24, 203.0.113.25", "note": "VPN"}))
            .await;
        opened.assert_status(StatusCode::OK);
        assert_eq!(
            opened.json()["sources"],
            "10.8.0.0/24,203.0.113.25/32",
            "canonical"
        );
        admin
            .send(
                Method::POST,
                &url("/blocked"),
                Some(&json!({"cidr": "0.0.0.0/0"})),
            )
            .await
            .assert_validation_error("cidr");
        admin
            .send(
                Method::POST,
                &url("/blocked"),
                Some(&json!({"cidr": "198.51.100.7"})),
            )
            .await
            .assert_status(StatusCode::OK);
        admin
            .send(
                Method::POST,
                &url("/blocked"),
                Some(&json!({"cidr": "198.51.100.7/32"})),
            )
            .await
            .assert_validation_error("cidr");
        // Each change recorded with it: changed again, made and removed.
        let id = opened.json()["id"].as_i64().unwrap();
        admin
            .send(Method::PATCH, &url(&format!("/openings/{id}")), Some(&json!({"ports": "5432", "protocol": "tcp", "target": "published", "sources_v4": "10.8.0.7/24, 203.0.113.25", "note": "VPN"})))
            .await
            .assert_status(StatusCode::OK);
        let extra = open(
            json!({"ports": "8080", "protocol": "tcp", "target": "published", "anywhere": true}),
        )
        .await;
        extra.assert_status(StatusCode::OK);
        let extra = extra.json()["id"].as_i64().unwrap();
        admin
            .send(Method::DELETE, &url(&format!("/openings/{extra}")), None)
            .await
            .assert_status(StatusCode::OK);
        let blocked = admin
            .send(
                Method::POST,
                &url("/blocked"),
                Some(&json!({"cidr": "192.0.2.9"})),
            )
            .await;
        blocked.assert_status(StatusCode::OK);
        let blocked = blocked.json()["id"].as_i64().unwrap();
        admin
            .send(Method::DELETE, &url(&format!("/blocked/{blocked}")), None)
            .await
            .assert_status(StatusCode::OK);
        admin
            .send(Method::DELETE, &url(&format!("/blocked/{blocked}")), None)
            .await
            .assert_status(StatusCode::NOT_FOUND);
        let events: Vec<String> = rustcms::models::audit_log::Entity::find()
            .all(state.db())
            .await
            .unwrap()
            .into_iter()
            .map(|a| a.event)
            .collect();
        for event in [
            "firewall.opening.create",
            "firewall.opening.update",
            "firewall.opening.delete",
            "firewall.blocked.create",
            "firewall.blocked.delete",
        ] {
            assert!(events.iter().any(|e| e == event), "{event}: {events:?}");
        }

        // The page: the host read, each port with what the firewall does.
        outputs
            .lock()
            .unwrap()
            .push_back((0, discovered("installed=no\n")));
        let page = firewall_service::report(state, &engine, true).await;
        let page = page.unwrap_or_else(|e| {
            panic!(
                "{e:?} calls={:?} left={:?}",
                calls.lock().unwrap(),
                outputs.lock().unwrap()
            )
        });
        assert_eq!(page["available"], true);
        assert_eq!(page["enabled"], false);
        let by_port = |p: u64| {
            page["ports"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["port"] == p)
                .cloned()
                .unwrap()
        };
        assert_eq!(by_port(22)["state"], "always_open");
        assert_eq!(by_port(2377)["owner"], "Docker Swarm");
        assert_eq!(by_port(5432)["state"], "open", "off: open as it is");
        // Without a domain or the tunnel, it may not be turned on.
        assert!(page["unsupported"].to_string().contains("no domain"));
        outputs
            .lock()
            .unwrap()
            .push_back((0, discovered("installed=no\n")));
        let refused = firewall_service::apply(state, &engine, &by, true)
            .await
            .unwrap_err();
        assert!(format!("{refused:?}").contains("domain"), "{refused:?}");
        state.service::<Traefik>().unwrap().set_tunnel(true);

        // Applied: installed, then journaled and loaded by the host.
        {
            let mut q = outputs.lock().unwrap();
            q.push_back((0, discovered("installed=no\n")));
            q.push_back((0, "installed\n".to_owned()));
            q.push_back((0, applied()));
        }
        let tx = firewall_service::apply(state, &engine, &by, true)
            .await
            .unwrap();
        let txid = tx["txid"].as_str().unwrap().to_owned();
        let row = firewall_transaction::Entity::find()
            .order_by_desc(firewall_transaction::Column::Id)
            .one(state.db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "pending");
        assert!(
            row.ruleset
                .contains("ct original proto-dst 5432 ip saddr @o")
        );
        assert!(row.ruleset.contains("elements = { 198.51.100.7/32 }"));
        assert!(row.ruleset.contains("\"docker0\""));
        let host_calls: Vec<String> = calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with("host: "))
            .cloned()
            .collect();
        assert!(
            host_calls.iter().any(|c| c.contains("set -eu")),
            "{host_calls:?}"
        );
        assert!(
            host_calls
                .iter()
                .any(|c| c.ends_with(&format!("fw.sh apply {txid} {}", row.id)))
        );
        // Then the bridges as found, for the host's own reloads.
        assert!(host_calls.last().unwrap().ends_with("fw.sh ifaces"));

        // One change at a time.
        outputs
            .lock()
            .unwrap()
            .push_back((0, "state=applied\nboot=b\nnow=b\ntimer=active\n".to_owned()));
        let busy = firewall_service::apply(state, &engine, &by, true)
            .await
            .unwrap_err();
        assert!(format!("{busy:?}").contains("waiting"), "{busy:?}");

        // Kept: the host still has the table it loaded.
        {
            let mut q = outputs.lock().unwrap();
            q.push_back((0, format!("installed=yes\nconfirmed=no\nlast={txid} applied\nboot_failed=none\n== table\n{TABLE}\n")));
            q.push_back((0, String::new()));
        }
        firewall_service::keep(state, &engine, &by, &txid, true)
            .await
            .unwrap();
        let row = firewall_transaction::Entity::find_by_id(row.id)
            .one(state.db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "kept");
        assert_eq!(
            panel_setting::get(state.db(), "firewall.confirmed_hash")
                .await
                .unwrap(),
            row.hash
        );
        // Kept twice: no longer waiting.
        let again = firewall_service::keep(state, &engine, &by, &txid, true)
            .await
            .unwrap_err();
        assert!(
            again.to_string().contains("no longer waiting")
                || format!("{again:?}").contains("no longer waiting")
        );

        // A second change, its table changed meanwhile: not kept.
        {
            let mut q = outputs.lock().unwrap();
            q.push_back((0, discovered(&format!("installed=yes\nconfirmed=yes\nlast={txid} kept\nboot_failed=none\n== table\n{TABLE}\n"))));
            q.push_back((0, "installed\n".to_owned()));
            q.push_back((0, applied()));
        }
        let second = firewall_service::apply(state, &engine, &by, true)
            .await
            .unwrap();
        let second = second["txid"].as_str().unwrap().to_owned();
        let other = TABLE.replace("\"drop\":null", "\"accept\":null");
        outputs.lock().unwrap().push_back((0, format!("installed=yes\nconfirmed=yes\nlast={second} reverted\nboot_failed=none\n== table\n{other}\n")));
        let stale = firewall_service::keep(state, &engine, &by, &second, true)
            .await
            .unwrap_err();
        assert!(format!("{stale:?}").contains("not the one"), "{stale:?}");
        // Reverted by the user.
        outputs.lock().unwrap().push_back((0, String::new()));
        outputs.lock().unwrap().push_back((
            0,
            "state=reverted\nboot=b\nnow=b\ntimer=inactive\n".to_owned(),
        ));
        firewall_service::revert(state, &engine, &by, &second, true)
            .await
            .unwrap();

        // An interrupted change: settled from the host's journal.
        {
            let mut q = outputs.lock().unwrap();
            q.push_back((0, discovered(&format!("installed=yes\nconfirmed=yes\nlast={second} reverted\nboot_failed=none\n== table\n{TABLE}\n"))));
            q.push_back((0, "installed\n".to_owned()));
            q.push_back((1, String::new()));
        }
        let failed = firewall_service::apply(state, &engine, &by, true)
            .await
            .unwrap_err();
        assert!(format!("{failed:?}").contains("refused"), "{failed:?}");
        let states: Vec<String> = firewall_transaction::Entity::find()
            .order_by_asc(firewall_transaction::Column::Id)
            .all(state.db())
            .await
            .unwrap()
            .into_iter()
            .map(|t| t.state)
            .collect();
        assert_eq!(states, ["kept", "reverted", "failed"]);

        // Off: the host's files and units removed.
        outputs.lock().unwrap().push_back((0, String::new()));
        firewall_service::disable(state, &engine, &by, true)
            .await
            .unwrap();
        assert!(
            calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.ends_with("fw.sh disable"))
        );
        // Every host command ran in a helper removed after it.
        let helpers = calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| c.starts_with("host helper"))
            .count();
        assert!(helpers >= 8, "{helpers}");
    }
);
