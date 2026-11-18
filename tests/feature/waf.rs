//! The WAF's settings: two changes at once (a save, an exception, the
//! rules' source) never lose one another.

use rdockpanel::database::migrations::Migrator;
use rdockpanel::services::waf_service::{Exception, Settings};
use rustcms::axum::http::StatusCode;
use rustcms_testing::{TestDb, db_test};
use serde_json::json;

use crate::support::{self, fake_docker::FakeDocker};

fn exception(rule: &str) -> Exception {
    Exception {
        rule: rule.to_owned(),
        host: "shop.example.com".to_owned(),
        since: None,
    }
}

db_test!(
    two_changes_at_once_keep_both,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let state = app.state().clone();
        // The first writer reads and holds the settings…
        let first = rustcms::db::begin_write(state.db()).await.unwrap();
        let mut settings = Settings::load_for_change(&first).await.unwrap();
        settings.exceptions.push(exception("942100"));
        // …while a second one starts its own change.
        let second = tokio::spawn({
            let state = state.clone();
            async move {
                let txn = rustcms::db::begin_write(state.db()).await.unwrap();
                let mut settings = Settings::load_for_change(&txn).await.unwrap();
                settings.exceptions.push(exception("941100"));
                settings.store(&txn).await.unwrap();
                txn.commit().await.unwrap();
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        settings.store(&first).await.unwrap();
        first.commit().await.unwrap();
        second.await.unwrap();
        let mut rules: Vec<String> = Settings::load(state.db())
            .await
            .unwrap()
            .exceptions
            .into_iter()
            .map(|e| e.rule)
            .collect();
        rules.sort();
        assert_eq!(rules, ["941100", "942100"]);
    }
);

db_test!(
    an_exceptions_wrong_domain_is_said_on_its_field,
    migrate = Migrator,
    |db: TestDb| async move {
        let app = support::app_with_docker(db, FakeDocker::default()).await;
        let root = support::user_with(&app, "root", &[]).await;
        app.make_super_admin(&root).await;
        let admin = app.acting_as(&root).await;
        let url = app.admin_api("x/apps/waf/exceptions");
        let refused = admin
            .post_json(&url, &json!({"rule": "942100", "host": "*.example.com"}))
            .await;
        refused.assert_status(StatusCode::UNPROCESSABLE_ENTITY);
        let body = refused.json();
        assert!(
            body["errors"]["host"].is_array() && body["errors"]["rule"].is_null(),
            "{body}"
        );
        let refused = admin
            .post_json(&url, &json!({"rule": "nope", "host": "shop.example.com"}))
            .await
            .json();
        assert!(
            refused["errors"]["rule"].is_array() && refused["errors"]["host"].is_null(),
            "{refused}"
        );
    }
);
