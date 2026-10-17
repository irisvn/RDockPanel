//! Helpers shared by the test suites.

#![allow(dead_code)]

pub mod fake_docker;

use rust_panel::bootstrap;
use rustcms_testing::{TestApp, TestDb};

/// The application on `db` (migrated by `db_test!`).
pub async fn app(db: TestDb) -> TestApp {
    TestApp::new(bootstrap::build, db).await
}
