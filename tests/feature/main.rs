//! Feature tests: the application through HTTP, on SQLite, PostgreSQL and
//! MySQL (`db_test!`). One test binary; `rustcms make:test <Name>` adds modules.

#[path = "../support/mod.rs"]
mod support;

mod apps;
mod backups;
mod compose;
mod databases;
mod deployments;
mod docker;
mod docker_routes;
mod health;
mod live;
mod notifications;
mod projects;
mod routing;
mod templates;
mod terminal;
// rustcms:tests
