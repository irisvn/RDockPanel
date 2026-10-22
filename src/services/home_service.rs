//! The homepage at `/`: what RDockPanel is, a link to GitHub, the way to
//! the panel. One static page in English or Vietnamese (`?lang=`, else the
//! browser's language), with nothing of this installation on it (no
//! version, address or project). Turned off in Settings → Panel: `/` then
//! leads to `/admin`.

use sea_orm::ConnectionTrait;

use crate::models::panel_setting;

const ENABLED_KEY: &str = "home.enabled";

const PAGE: &str = include_str!("../../resources/home/index.html");
pub const CSS: &str = include_str!("../../resources/home/home.css");
pub const FAVICON: &str = include_str!("../../resources/home/favicon.svg");
pub const SCRIPT: &str = include_str!("../../resources/home/home.js");

/// Its images, by name: `/home/img/{name}`.
pub const IMAGES: [(&str, &[u8]); 8] = [
    (
        "dashboard.jpg",
        include_bytes!("../../resources/home/img/dashboard.jpg"),
    ),
    (
        "dashboard-dark.jpg",
        include_bytes!("../../resources/home/img/dashboard-dark.jpg"),
    ),
    (
        "service.jpg",
        include_bytes!("../../resources/home/img/service.jpg"),
    ),
    (
        "templates.jpg",
        include_bytes!("../../resources/home/img/templates.jpg"),
    ),
    (
        "deployments.jpg",
        include_bytes!("../../resources/home/img/deployments.jpg"),
    ),
    (
        "domains.jpg",
        include_bytes!("../../resources/home/img/domains.jpg"),
    ),
    (
        "containers.jpg",
        include_bytes!("../../resources/home/img/containers.jpg"),
    ),
    (
        "actions.jpg",
        include_bytes!("../../resources/home/img/actions.jpg"),
    ),
];

/// The panel's primary color (Settings → Branding) for the page, none
/// set: its own. Checked when saved; only a color's characters here still.
pub async fn brand_css<C: ConnectionTrait>(db: &C) -> Result<String, sea_orm::DbErr> {
    let primary = panel_setting::get(db, crate::services::branding_service::PRIMARY_KEY).await?;
    Ok(match primary {
        Some(color)
            if !color.is_empty()
                && color
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "#(),.% ".contains(c)) =>
        {
            format!(":root {{ --primary: {color}; }}\n")
        }
        _ => String::new(),
    })
}

/// Whether `/` shows the homepage (never set: it does).
pub async fn enabled<C: ConnectionTrait>(db: &C) -> Result<bool, sea_orm::DbErr> {
    Ok(panel_setting::get(db, ENABLED_KEY).await?.as_deref() != Some("false"))
}

pub async fn set_enabled<C: ConnectionTrait>(db: &C, on: bool) -> Result<(), sea_orm::DbErr> {
    panel_setting::set(db, ENABLED_KEY, if on { "true" } else { "false" }).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Vi,
}

impl Lang {
    /// `?lang=` first, else the browser's: of English and Vietnamese, the
    /// one it prefers (`q`), English when it names neither.
    pub fn pick(query: Option<&str>, accept_language: Option<&str>) -> Lang {
        let of = |tag: &str| {
            let tag = tag.trim().to_ascii_lowercase();
            if tag == "vi" || tag.starts_with("vi-") {
                Some(Lang::Vi)
            } else if tag == "en" || tag.starts_with("en-") {
                Some(Lang::En)
            } else {
                None
            }
        };
        if let Some(lang) = query.and_then(of) {
            return lang;
        }
        let mut best: Option<(f32, Lang)> = None;
        for item in accept_language.unwrap_or("").split(',') {
            let mut parts = item.split(';');
            let Some(lang) = of(parts.next().unwrap_or("")) else {
                continue;
            };
            let q = parts
                .find_map(|p| p.trim().strip_prefix("q="))
                .map_or(Some(1.0), |q| q.trim().parse::<f32>().ok())
                .unwrap_or(0.0);
            // The first of equal ones wins; `q=0` is "not this one".
            if q > 0.0 && best.is_none_or(|(b, _)| q > b) {
                best = Some((q, lang));
            }
        }
        best.map_or(Lang::En, |(_, lang)| lang)
    }
}

/// Names the assets' contents (their cache is a day): not the panel's
/// version, which this page never tells.
pub fn assets_tag() -> &'static str {
    static TAG: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (CSS, FAVICON, SCRIPT).hash(&mut hasher);
        for (name, bytes) in IMAGES {
            (name, bytes).hash(&mut hasher);
        }
        format!("{:x}", hasher.finish())
    });
    &TAG
}

/// The page in `lang`; `v` busts its assets' cache ([`assets_tag`]).
pub fn render(lang: Lang, v: &str) -> String {
    let texts = match lang {
        Lang::En => EN,
        Lang::Vi => VI,
    };
    let mut page = PAGE.replace("{{v}}", &escape(v));
    for (key, text) in texts {
        page = page.replace(&format!("{{{{{key}}}}}"), &escape(text));
    }
    page
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

type Texts = &'static [(&'static str, &'static str)];

const EN: Texts = &[
    ("lang", "en"),
    ("title", "RDockPanel: a free, self-hosted server panel"),
    (
        "description",
        "Monitor your server, manage Docker and deploy apps on Docker Swarm with domains and HTTPS. Free and open source, built in Rust.",
    ),
    ("skip", "Skip to content"),
    ("nav_main", "Main"),
    ("nav_footer", "Footer"),
    ("nav_features", "Features"),
    ("nav_screens", "Screens"),
    ("nav_install", "Install"),
    ("other_href", "/?lang=vi"),
    ("other_lang", "vi"),
    ("other_label", "Tiếng Việt"),
    ("sign_in", "Sign in"),
    ("eyebrow", "Free · Self-hosted · Built in Rust"),
    ("headline", "Your server. Your apps. One panel."),
    (
        "lead",
        "Monitor your server, manage Docker, and deploy apps on Docker Swarm with domains and HTTPS through Traefik. Every feature is free: no project limits, no paid tiers.",
    ),
    ("view_github", "View on GitHub"),
    ("install_cta", "Install RDockPanel"),
    (
        "cue",
        "Manage this server? Sign in. Exploring RDockPanel? See it on GitHub or install it on your own server.",
    ),
    (
        "alt_dashboard",
        "The RDockPanel dashboard: the server's CPU, memory, disk and network, and a project with its services",
    ),
    ("features_title", "Everything a small server needs"),
    ("features_sub", "Open source and free, every feature."),
    ("f1_title", "Server monitoring"),
    (
        "f1",
        "CPU, memory, disk and network, live and with their recent history.",
    ),
    ("f2_title", "Docker management"),
    (
        "f2",
        "Containers, images, volumes and networks, with live logs and a terminal in the browser.",
    ),
    ("f3_title", "App deployments"),
    (
        "f3",
        "Deploy an image or build from Git with its Dockerfile, with history and rollback.",
    ),
    ("f4_title", "Domains and HTTPS"),
    (
        "f4",
        "Domains through Traefik with Let's Encrypt, redirects, rate limits, basic auth and maintenance pages.",
    ),
    ("f5_title", "Databases and templates"),
    (
        "f5",
        "PostgreSQL, MySQL, MariaDB, MongoDB and Redis in one click, or 800+ EasyPanel templates.",
    ),
    ("f6_title", "Scheduled backups"),
    (
        "f6",
        "Databases and volumes to a local directory or S3, with retention and restore.",
    ),
    ("f7_title", "Team access"),
    (
        "f7",
        "Roles, project members and an audit log of who did what.",
    ),
    ("f8_title", "Automation and alerts"),
    (
        "f8",
        "Deploy from CI or GitHub webhooks; alerts on Discord, Slack, Telegram, email and webhooks.",
    ),
    ("screens_title", "See it"),
    ("screens_sub", "The panel as it is, from a real server."),
    ("s1", "A service: its state, Deploy and its tabs"),
    ("s2", "800+ templates, one click each"),
    ("s3", "Deployments, their logs and rollback"),
    ("s4", "Every domain and where it leads"),
    ("s5", "Every container, with logs and a terminal"),
    ("s6", "Actions: what was done, and by whom"),
    ("install_title", "Run it on your own server"),
    (
        "install_sub",
        "A fresh Ubuntu or Debian server (amd64 or arm64), as root:",
    ),
    (
        "install_note",
        "Replace you@example.com with the administrator's email. More in the",
    ),
    ("install_guide", "installation guide"),
    ("early", "Early development: try it on a test server first."),
    ("footer", "RDockPanel · free and open source"),
    ("docs", "Docs"),
    ("viewer", "Screens"),
    ("previous", "Previous"),
    ("next", "Next"),
    ("close", "Close"),
];

const VI: Texts = &[
    ("lang", "vi"),
    (
        "title",
        "RDockPanel: bảng điều khiển máy chủ miễn phí, tự cài đặt",
    ),
    (
        "description",
        "Giám sát máy chủ, quản lý Docker và triển khai ứng dụng trên Docker Swarm với tên miền và HTTPS. Miễn phí, mã nguồn mở, viết bằng Rust.",
    ),
    ("skip", "Đến nội dung chính"),
    ("nav_main", "Chính"),
    ("nav_footer", "Chân trang"),
    ("nav_features", "Tính năng"),
    ("nav_screens", "Hình ảnh"),
    ("nav_install", "Cài đặt"),
    ("other_href", "/?lang=en"),
    ("other_lang", "en"),
    ("other_label", "English"),
    ("sign_in", "Đăng nhập"),
    ("eyebrow", "Miễn phí · Tự cài đặt · Viết bằng Rust"),
    (
        "headline",
        "Máy chủ của bạn. Ứng dụng của bạn. Một bảng điều khiển.",
    ),
    (
        "lead",
        "Giám sát máy chủ, quản lý Docker và triển khai ứng dụng trên Docker Swarm, có tên miền và HTTPS qua Traefik. Mọi tính năng đều miễn phí: không giới hạn dự án, không có gói trả phí.",
    ),
    ("view_github", "Xem trên GitHub"),
    ("install_cta", "Cài RDockPanel"),
    (
        "cue",
        "Bạn quản lý máy chủ này? Hãy đăng nhập. Đang tìm hiểu RDockPanel? Xem trên GitHub hoặc cài lên máy chủ của bạn.",
    ),
    (
        "alt_dashboard",
        "Bảng điều khiển RDockPanel: CPU, bộ nhớ, ổ đĩa, mạng của máy chủ và một dự án với các dịch vụ của nó",
    ),
    ("features_title", "Đủ mọi thứ cho một máy chủ nhỏ"),
    ("features_sub", "Mã nguồn mở và miễn phí, mọi tính năng."),
    ("f1_title", "Giám sát máy chủ"),
    (
        "f1",
        "CPU, bộ nhớ, ổ đĩa và mạng, theo thời gian thực kèm lịch sử gần đây.",
    ),
    ("f2_title", "Quản lý Docker"),
    (
        "f2",
        "Container, image, volume và network, có log trực tiếp và terminal ngay trên trình duyệt.",
    ),
    ("f3_title", "Triển khai ứng dụng"),
    (
        "f3",
        "Chạy một image hoặc build từ Git bằng Dockerfile, có lịch sử và rollback.",
    ),
    ("f4_title", "Tên miền và HTTPS"),
    (
        "f4",
        "Tên miền qua Traefik với Let's Encrypt, chuyển hướng, giới hạn request, basic auth và trang bảo trì.",
    ),
    ("f5_title", "Database và template"),
    (
        "f5",
        "PostgreSQL, MySQL, MariaDB, MongoDB và Redis chỉ một cú bấm, hoặc hơn 800 template của EasyPanel.",
    ),
    ("f6_title", "Sao lưu theo lịch"),
    (
        "f6",
        "Database và volume ra thư mục trên máy hoặc S3, có giữ bản cũ và khôi phục.",
    ),
    ("f7_title", "Làm việc nhóm"),
    ("f7", "Vai trò, thành viên dự án và nhật ký ai đã làm gì."),
    ("f8_title", "Tự động hoá và thông báo"),
    (
        "f8",
        "Triển khai từ CI hay webhook GitHub; thông báo qua Discord, Slack, Telegram, email và webhook.",
    ),
    ("screens_title", "Hình ảnh"),
    (
        "screens_sub",
        "Giao diện thật, chụp từ một máy chủ đang chạy.",
    ),
    ("s1", "Một dịch vụ: trạng thái, nút Deploy và các tab"),
    ("s2", "Hơn 800 template, mỗi cái một cú bấm"),
    ("s3", "Các lần triển khai, log và rollback"),
    ("s4", "Mọi tên miền và nơi chúng trỏ tới"),
    ("s5", "Mọi container, có log và terminal"),
    ("s6", "Hoạt động: việc gì đã làm, và ai làm"),
    ("install_title", "Cài lên máy chủ của bạn"),
    (
        "install_sub",
        "Một máy chủ Ubuntu hoặc Debian mới (amd64 hoặc arm64), chạy với quyền root:",
    ),
    (
        "install_note",
        "Thay you@example.com bằng email của quản trị viên. Xem thêm trong",
    ),
    ("install_guide", "hướng dẫn cài đặt"),
    (
        "early",
        "Đang phát triển: hãy thử trên một máy chủ thử nghiệm trước.",
    ),
    ("footer", "RDockPanel · miễn phí, mã nguồn mở"),
    ("docs", "Tài liệu"),
    ("viewer", "Hình ảnh"),
    ("previous", "Ảnh trước"),
    ("next", "Ảnh sau"),
    ("close", "Đóng"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_placeholder_is_filled_in_both_languages() {
        for lang in [Lang::En, Lang::Vi] {
            let page = render(lang, "1.2.3");
            assert!(
                !page.contains("{{"),
                "{lang:?}: {}",
                &page[page.find("{{").unwrap()..][..40]
            );
            assert!(page.contains("/home/home.css?v=1.2.3"));
        }
        assert!(render(Lang::Vi, "x").contains(r#"<html lang="vi">"#));
    }

    #[test]
    fn the_language() {
        assert_eq!(Lang::pick(Some("vi"), Some("en-US")), Lang::Vi);
        assert_eq!(Lang::pick(Some("en"), Some("vi-VN,vi")), Lang::En);
        assert_eq!(Lang::pick(None, Some("vi-VN,vi;q=0.9,en;q=0.8")), Lang::Vi);
        assert_eq!(Lang::pick(None, Some("en-US,vi;q=0.5")), Lang::En);
        assert_eq!(Lang::pick(Some("fr"), None), Lang::En);
        assert_eq!(Lang::pick(None, Some("vi;q=0,en;q=1")), Lang::En);
        assert_eq!(Lang::pick(None, Some("fr,vi;q=0.9,en;q=0.8")), Lang::Vi);
        assert_eq!(Lang::pick(None, Some("vietnam")), Lang::En);
        assert!(!assets_tag().contains('.'), "no version");
        assert_eq!(Lang::pick(None, None), Lang::En);
    }

    #[test]
    fn its_images_are_jpegs() {
        for (name, bytes) in IMAGES {
            assert!(bytes.starts_with(&[0xFF, 0xD8]), "{name}");
        }
    }
}
