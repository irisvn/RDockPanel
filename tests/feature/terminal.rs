//! The container terminal (WebSocket): what it carries, who may open it, and
//! its entry in the actions log.

use std::time::Duration;

use rdockpanel::database::migrations::Migrator;
use rdockpanel::models::action;
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestDb, db_test};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::support::{self, fake_docker::FakeDocker};

const OPERATOR: &[&str] = &["docker.view", "docker.manage", "docker.exec"];
const WAIT: Duration = Duration::from_secs(5);

fn exec(container: &str) -> String {
    format!("/admin/api/x/docker/containers/{container}/exec?cols=100&rows=30")
}

db_test!(
    a_terminal_carries_bytes_and_resizes,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::with_app_and_panel();
        let app = support::app_with_docker(db, fake.clone()).await;
        let operator = support::user_with(&app, "operator", OPERATOR).await;
        let client = app.acting_as(&operator).await;
        client.fetch_csrf().await;
        let mut socket = client
            .socket(&exec("web"))
            .await
            .expect("the terminal opens");

        let prompt = socket.next_data(WAIT).await.expect("a prompt").into_data();
        assert_eq!(&prompt[..], b"web$ ");
        socket.send_binary(b"ls -la\n").await;
        let echoed = socket
            .next_data(WAIT)
            .await
            .expect("the shell's output")
            .into_data();
        assert_eq!(&echoed[..], b"ls -la\n");

        socket
            .send_text(r#"{"type":"resize","cols":120,"rows":40}"#)
            .await;
        socket.send_text("not a command").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let calls = fake.calls();
        assert_eq!(calls[0], "exec id-web 100x30");
        let token = calls[1]
            .strip_prefix("token ")
            .expect("the session's mark")
            .to_owned();
        assert_eq!(calls[2], "resize exec-web 120x40");

        // Closing ends the shell, and the session's action with how long it lasted.
        drop(socket);
        for _ in 0..50 {
            if fake
                .calls()
                .iter()
                .any(|c| *c == format!("end exec id-web {token}"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // Only this session's processes, by its mark.
        assert!(
            fake.calls().contains(&format!("end exec id-web {token}")),
            "{:?}",
            fake.calls()
        );
        let db = app.state().db();
        let mut row = None;
        for _ in 0..50 {
            row = action::Entity::find()
                .filter(action::Column::Kind.eq("container.exec"))
                .one(db)
                .await
                .unwrap();
            if row.as_ref().is_some_and(|r| r.status != action::RUNNING) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let row = row.expect("an action row");
        assert_eq!(
            (row.target.as_str(), row.status.as_str()),
            ("id-web", action::OK)
        );
        assert!(
            row.log
                .as_deref()
                .unwrap_or_default()
                .starts_with("session lasted"),
            "{row:?}"
        );
    }
);

db_test!(
    a_terminal_needs_docker_exec_and_system_manage_for_the_panel,
    migrate = Migrator,
    |db: TestDb| async move {
        let fake = FakeDocker::with_app_and_panel();
        let app = support::app_with_docker(db, fake.clone()).await;
        let viewer = support::user_with(&app, "viewer", &["docker.view", "docker.manage"]).await;
        let operator = support::user_with(&app, "operator", OPERATOR).await;
        let admin = support::user_with(&app, "admin", &[]).await;
        app.make_super_admin(&admin).await;

        let refused = |status: StatusCode| {
            move |result: Result<rustcms_testing::TestSocket, rustcms_testing::Refused>| {
                assert_eq!(result.err().map(|r| r.status), Some(status));
            }
        };
        let viewer = app.acting_as(&viewer).await;
        viewer.fetch_csrf().await;
        refused(StatusCode::FORBIDDEN)(viewer.socket(&exec("web")).await);

        let operator = app.acting_as(&operator).await;
        operator.fetch_csrf().await;
        refused(StatusCode::FORBIDDEN)(operator.socket(&exec("panel")).await);
        refused(StatusCode::NOT_FOUND)(operator.socket(&exec("nope")).await);
        refused(StatusCode::NOT_FOUND)(operator.socket(&exec("..%2Fx")).await);
        // Without the CSRF subprotocol the handshake is refused.
        refused(StatusCode::FORBIDDEN)(
            operator
                .socket_with(&exec("web"), Some("rustcms.v1"), &[])
                .await,
        );

        let admin = app.acting_as(&admin).await;
        admin.fetch_csrf().await;
        let mut socket = admin
            .socket(&exec("panel"))
            .await
            .expect("system.manage opens the panel's own");
        assert!(socket.next_data(WAIT).await.is_some());
        assert!(
            !fake.calls().iter().any(|c| c.contains("id-web")),
            "{:?}",
            fake.calls()
        );
    }
);

db_test!(
    terminals_cut_by_a_stop_end_at_boot,
    migrate = Migrator,
    |db: TestDb| async move {
        use chrono::Utc;
        use rdockpanel::http::controllers::docker_controller::TERMINAL_SESSION;
        use rdockpanel::services::action_service::interrupted_terminals;
        use sea_orm::{ActiveModelTrait, Set};
        let app = support::app_with_docker(db, FakeDocker::with_app_and_panel()).await;
        let db = app.state().db();
        let running = action::ActiveModel {
            kind: Set("container.exec".to_owned()),
            target: Set("id-web".to_owned()),
            status: Set(action::RUNNING.to_owned()),
            log: Set(Some(format!("{TERMINAL_SESSION}abc123"))),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            ..Default::default()
        }
        .insert(db)
        .await
        .unwrap();

        assert_eq!(
            interrupted_terminals(db, TERMINAL_SESSION).await.unwrap(),
            [("id-web".to_owned(), "abc123".to_owned())]
        );
        let row = action::Entity::find_by_id(running.id)
            .one(db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, action::FAILED);
        assert!(
            interrupted_terminals(db, TERMINAL_SESSION)
                .await
                .unwrap()
                .is_empty()
        );
    }
);
