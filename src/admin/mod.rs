//! The admin panel (like `app/Filament`): `resources` (one per model) and
//! `pages` (settings pages).

pub mod pages;
pub mod resources;

use rustcms::admin::Data;
use serde_json::Value;

/// The integer `field` of `data` (keys and 64-bit integers travel as strings).
pub(crate) fn int(data: &Data, field: &str) -> Option<i64> {
    match data.get(field)? {
        Value::String(text) => text.parse().ok(),
        value => value.as_i64(),
    }
}

/// The text `field` of `data` (`None` when absent or null).
pub(crate) fn text<'a>(data: &'a Data, field: &str) -> Option<&'a str> {
    data.get(field)?.as_str()
}

/// Whether `field` of `data` is set (present and not null).
pub(crate) fn is_set(data: &Data, field: &str) -> bool {
    data.get(field).is_some_and(|value| !value.is_null())
}
