//! rdockpanel: a rustcms application. Where things live:
//! `bootstrap` assembles the app from `providers`; `routes` declares the API,
//! served by `http`; `models`, `services` and `policies` hold the domain;
//! `admin` the panel; `database` migrations, seeders and factories.
// The service page's `json!` (`apps_controller::service`) is deep.
#![recursion_limit = "256"]

pub mod admin;
pub mod bootstrap;
pub mod database;
pub mod http;
pub mod jobs;
pub mod models;
pub mod policies;
pub mod providers;
pub mod routes;
pub mod services;
// rustcms:mods
