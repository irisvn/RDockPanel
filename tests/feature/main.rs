//! Feature tests: the application through HTTP, on SQLite, PostgreSQL and
//! MySQL (`db_test!`). One test binary; `rustcms make:test <Name>` adds modules.

#[path = "../support/mod.rs"]
mod support;

mod api;
mod apps;
mod backups;
mod certificates;
mod compose;
mod databases;
mod deployments;
mod docker;
mod docker_routes;
mod domains;
mod git;
mod health;
mod live;
mod members;
mod notifications;
mod projects;
mod routing;
mod system;
mod templates;
mod terminal;
// rustcms:tests
