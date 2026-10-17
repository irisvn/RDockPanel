//! The admin panel at `/admin` (like a Filament panel provider): its
//! resources (`rustcms make:resource`), pages (`make:settings`) and labels.

use rustcms::prelude::*;

pub fn provider() -> AdminProvider {
    AdminProvider::new()
        .brand("rust-panel")
        // Labels in Vietnamese (`rustcms lang:missing vi` lists what is missing).
        .translations("vi", include_str!("../../lang/vi.toml"))
    // rustcms:resources
    // rustcms:pages
    // rustcms:extensions
}
