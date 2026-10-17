# resources

Files the application embeds for its admin panel:

- `views/admin/*.html`: templates of custom pages and slots (`rustcms make:page`, `view!("file.html")`); with
  `APP_ENV=local` and `cargo run`, edits show on reload.
- `admin/js/<vendor>/*.js`: extension scripts (widgets, script pages, cells; `rustcms make:widget`, `make:page --script`).
- `admin/css/*.css`: extension stylesheets (`AdminProvider::extension_style`).
- `admin/*.png`: logo and favicon (`Theme::logo`, `Theme::favicon`).
