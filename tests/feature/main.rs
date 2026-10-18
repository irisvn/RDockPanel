//! Feature tests: the application through HTTP, on SQLite, PostgreSQL and
//! MySQL (`db_test!`). One test binary; `rustcms make:test <Name>` adds modules.

#[path = "../support/mod.rs"]
mod support;

mod docker;
mod docker_routes;
mod health;
mod live;
mod projects;
mod terminal;
// rustcms:tests
