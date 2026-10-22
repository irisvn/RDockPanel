//! A service's maintenance page: what its domains show (503) while it is in
//! maintenance. The built-in page (title, subtitle, when it is back, a
//! contact, a logo), the panel's shared page (Settings) or the service's own
//! HTML, with `{{…}}` filled in.
//!
//! Every page runs in a sandbox (its CSP: no script, no form, an opaque
//! origin), and only on the service's own domains: a user's HTML never
//! reaches the panel's origin.

use chrono::{DateTime, FixedOffset, Utc};

use crate::models::{branding_image, panel_setting, service, service_logo};
pub use crate::services::home_service::Lang;

/// The panel's shared page and its default contact (Settings).
pub const TEMPLATE_KEY: &str = "maintenance.template";
pub const CONTACT_KEY: &str = "maintenance.contact";
/// The largest HTML written (bytes), page sent, and logo kept.
pub const MAX_HTML: usize = 256 * 1024;
pub const MAX_PAGE: usize = 1024 * 1024;
pub const MAX_LOGO: usize = 256 * 1024;
/// A logo larger than this stays out of the built-in page.
const MAX_EMBEDDED_LOGO: usize = 128 * 1024;
/// How many times one page may embed the logo.
const MAX_LOGOS: usize = 4;
/// The pages' policy: styles and images (inline, data, https), nothing run.
pub const CSP: &str = "sandbox; default-src 'none'; style-src 'unsafe-inline' https:; \
     img-src data: https:; font-src data: https:; form-action 'none'; base-uri 'none'; \
     frame-ancestors 'none'";
/// A preview's: the same, shown in the panel's frame.
pub const PREVIEW_CSP: &str = "sandbox; default-src 'none'; style-src 'unsafe-inline' https:; \
     img-src data: https:; font-src data: https:; form-action 'none'; base-uri 'none'; \
     frame-ancestors 'self'";

/// What a page says.
#[derive(Debug, Clone)]
pub struct Facts {
    pub title: String,
    pub subtitle: String,
    /// When it is back, and the offset it is shown in (minutes east of UTC).
    pub eta: Option<(DateTime<Utc>, i16)>,
    pub contact: Option<String>,
    /// Data URLs of its logo (light, dark: the light one when it has one).
    pub logo: Option<String>,
    pub logo_dark: Option<String>,
    pub service: String,
    pub project: String,
    /// The visitor's language (the built-in words).
    pub lang: Lang,
}

impl Facts {
    /// A service's, as saved (`contact`: the panel's when it has none).
    pub fn of(
        row: &service::Model,
        project: &str,
        contact: Option<String>,
        logo: (Option<String>, Option<String>),
    ) -> Facts {
        Facts {
            title: row.maintenance_title.clone().unwrap_or_default(),
            subtitle: row.maintenance_subtitle.clone().unwrap_or_default(),
            eta: row
                .maintenance_eta
                .map(|at| (at, offset(row.maintenance_eta_offset))),
            contact: row
                .maintenance_contact
                .clone()
                .filter(|c| !c.trim().is_empty())
                .or(contact),
            logo: logo.0,
            logo_dark: logo.1,
            service: row.name.clone(),
            project: project.to_owned(),
            lang: Lang::En,
        }
    }

    fn title(&self) -> &str {
        Some(self.title.trim())
            .filter(|t| !t.is_empty())
            .unwrap_or(match self.lang {
                Lang::En => "Under maintenance",
                Lang::Vi => "Đang bảo trì",
            })
    }

    fn subtitle(&self) -> &str {
        Some(self.subtitle.trim())
            .filter(|t| !t.is_empty())
            .unwrap_or(match self.lang {
                Lang::En => "We will be back soon.",
                Lang::Vi => "Chúng tôi sẽ sớm trở lại.",
            })
    }
}

impl Default for Facts {
    fn default() -> Self {
        Facts {
            title: String::new(),
            subtitle: String::new(),
            eta: None,
            contact: None,
            logo: None,
            logo_dark: None,
            service: String::new(),
            project: String::new(),
            lang: Lang::En,
        }
    }
}

/// A saved offset (minutes east of UTC), within the world's.
fn offset(minutes: Option<i32>) -> i16 {
    minutes
        .filter(|m| (-840..=840).contains(m))
        .and_then(|m| i16::try_from(m).ok())
        .unwrap_or(0)
}

/// `text` safe in HTML text and attribute values.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// When it is back, in its offset: `14:30, 22/10/2026 (GMT+7)`.
pub fn eta_text(at: DateTime<Utc>, offset: i16) -> String {
    let zone =
        FixedOffset::east_opt(i32::from(offset) * 60).unwrap_or(FixedOffset::east_opt(0).unwrap());
    let local = at.with_timezone(&zone);
    let (hours, minutes) = (offset.abs() / 60, offset.abs() % 60);
    let sign = if offset < 0 { '-' } else { '+' };
    let gmt = match (offset, minutes) {
        (0, _) => "GMT".to_owned(),
        (_, 0) => format!("GMT{sign}{hours}"),
        _ => format!("GMT{sign}{hours}:{minutes:02}"),
    };
    format!(
        "{}, {} ({gmt})",
        local.format("%H:%M"),
        local.format("%d/%m/%Y")
    )
}

/// `Retry-After` (seconds): until it is back, at least a minute and at most
/// a day; five minutes when nobody said.
pub fn retry_after(eta: Option<DateTime<Utc>>, now: DateTime<Utc>) -> i64 {
    match eta {
        Some(at) => (at - now).num_seconds().clamp(60, 86_400),
        None => 300,
    }
}

/// A contact as written: an email, or an https link.
pub fn valid_contact(text: &str) -> bool {
    let text = text.trim();
    if text.len() > 255 || text.chars().any(char::is_whitespace) {
        return false;
    }
    if let Some(rest) = text.strip_prefix("https://") {
        return !rest.is_empty() && !text.contains(['"', '<', '>', '\'', '`']);
    }
    let Some((user, host)) = text.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && !host.contains('@')
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !text.contains(['"', '<', '>', '\'', '`', '/', '\\'])
}

/// Where a contact leads (an https link; an email stays text: the sandbox
/// opens no mail program).
pub fn contact_href(contact: Option<&str>) -> String {
    contact
        .filter(|c| c.starts_with("https://"))
        .map(str::to_owned)
        .unwrap_or_default()
}

/// The placeholders a page may use.
pub const PLACEHOLDERS: &[&str] = &[
    "title",
    "subtitle",
    "eta",
    "eta_iso",
    "contact",
    "contact_href",
    "logo",
    "service",
    "project",
];

/// `template` with its placeholders filled in (escaped; an unknown one left
/// as written); `None` when the page would grow past [`MAX_PAGE`] or embed
/// the logo more than [`MAX_LOGOS`] times.
pub fn render(template: &str, facts: &Facts) -> Option<String> {
    let value = |name: &str| -> Option<String> {
        Some(match name {
            "title" => escape(facts.title()),
            "subtitle" => escape(facts.subtitle()),
            "eta" => facts
                .eta
                .map(|(at, offset)| escape(&eta_text(at, offset)))
                .unwrap_or_default(),
            "eta_iso" => facts.eta.map(|(at, _)| at.to_rfc3339()).unwrap_or_default(),
            "contact" => escape(facts.contact.as_deref().unwrap_or_default()),
            "contact_href" => escape(&contact_href(facts.contact.as_deref())),
            "logo" => facts.logo.clone().unwrap_or_default(),
            "service" => escape(&facts.service),
            "project" => escape(&facts.project),
            _ => return None,
        })
    };
    let mut out = String::with_capacity(template.len().min(MAX_PAGE));
    let mut rest = template;
    let mut logos = 0;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let name = after[..end].trim();
        match value(name) {
            Some(v) => {
                if name == "logo" {
                    logos += 1;
                    if logos > MAX_LOGOS {
                        return None;
                    }
                }
                out.push_str(&v);
            }
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
        if out.len() > MAX_PAGE {
            return None;
        }
    }
    out.push_str(rest);
    (out.len() <= MAX_PAGE).then_some(out)
}

/// The built-in page: the logo (when small enough), the title, the subtitle,
/// when it is back and the contact.
pub fn builtin(facts: &Facts) -> String {
    let small = |l: &Option<String>| {
        l.clone()
            .filter(|l| l.len() <= MAX_EMBEDDED_LOGO * 4 / 3 + 64)
    };
    let (light, dark) = (small(&facts.logo), small(&facts.logo_dark));
    let logo = match (&light, &dark) {
        (Some(l), Some(d)) => {
            format!(r#"<img class="light" src="{l}" alt=""><img class="dark" src="{d}" alt="">"#)
        }
        (Some(l), None) | (None, Some(l)) => format!(r#"<img src="{l}" alt="">"#),
        (None, None) => String::new(),
    };
    let mut meta = vec![];
    if let Some((at, offset)) = facts.eta {
        let back = match facts.lang {
            Lang::En => "Back at",
            Lang::Vi => "Dự kiến trở lại lúc",
        };
        meta.push(format!("{back} {}", escape(&eta_text(at, offset))));
    }
    if let Some(contact) = facts.contact.as_deref() {
        let href = contact_href(Some(contact));
        meta.push(if href.is_empty() {
            escape(contact)
        } else {
            format!(r#"<a href="{0}">{1}</a>"#, escape(&href), escape(contact))
        });
    }
    let meta = if meta.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="meta">{}</p>"#, meta.join(" · "))
    };
    let title = escape(facts.title());
    let subtitle = escape(facts.subtitle());
    let lang = match facts.lang {
        Lang::En => "en",
        Lang::Vi => "vi",
    };
    format!(
        r#"<!doctype html><html lang="{lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><title>{title}</title><style>{STYLE}</style></head><body><main>{logo}<h1>{title}</h1><p>{subtitle}</p>{meta}</main></body></html>"#
    )
}

const STYLE: &str = ":root{--bg:#fafafa;--fg:#171717;--muted:#737373}\
@media (prefers-color-scheme:dark){:root{--bg:#111;--fg:#ededed;--muted:#a1a1a1}.light{display:none}}\
@media not (prefers-color-scheme:dark){.dark{display:none}}\
body{margin:0;min-height:100vh;display:grid;place-items:center;padding:24px;box-sizing:border-box;\
font:18px/1.6 system-ui,-apple-system,\"Segoe UI\",Roboto,sans-serif;background:var(--bg);color:var(--fg);text-align:center}\
main{max-width:560px}img{height:40px;max-width:100%;margin-bottom:32px}\
h1{font-size:34px;font-weight:600;margin:0 0 12px;text-wrap:balance}p{margin:0;color:var(--muted)}\
.meta{margin-top:28px;font-size:16px;overflow-wrap:anywhere}a{color:inherit}\
@media (max-width:480px){body{font-size:17px}h1{font-size:28px}}";

/// HTML to start a page of one's own from: the built-in page with its
/// placeholders.
pub fn starter() -> String {
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>{{{{title}}}}</title>\n<style>\n{}\n</style>\n</head>\n<body>\n<main>\n  <img src=\"{{{{logo}}}}\" alt=\"\">\n  <h1>{{{{title}}}}</h1>\n  <p>{{{{subtitle}}}}</p>\n  <p class=\"meta\">{{{{eta}}}} · {{{{contact}}}}</p>\n</main>\n</body>\n</html>\n",
        STYLE.replace('}', "}\n").trim_end()
    )
}

/// The page: `template` (the service's own HTML, else the panel's shared
/// one) filled in, or the built-in one (none, or too large once filled).
pub fn page(template: Option<&str>, facts: &Facts) -> String {
    template
        .filter(|t| !t.trim().is_empty())
        .and_then(|t| render(t, facts))
        .unwrap_or_else(|| builtin(facts))
}

/// The template a service's page uses: its own HTML, or the panel's shared
/// page (`None`: the built-in one).
pub async fn template_of<C: sea_orm::ConnectionTrait>(
    db: &C,
    row: &service::Model,
) -> Result<Option<String>, sea_orm::DbErr> {
    if row.maintenance_page == "custom" {
        return Ok(row
            .maintenance_html
            .clone()
            .filter(|h| !h.trim().is_empty()));
    }
    Ok(panel_setting::get(db, TEMPLATE_KEY)
        .await?
        .filter(|t| !t.trim().is_empty()))
}

/// A service's logo as data URLs: its own, else the panel's (Branding:
/// light and dark).
pub async fn logo_of<C: sea_orm::ConnectionTrait>(
    db: &C,
    service_id: Option<i64>,
) -> Result<(Option<String>, Option<String>), sea_orm::DbErr> {
    use base64::Engine;
    use sea_orm::EntityTrait;
    let url = |mime: &str, data: &[u8]| {
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(data)
        )
    };
    if let Some(id) = service_id
        && let Some(own) = service_logo::Entity::find_by_id(id).one(db).await?
    {
        return Ok((Some(url(&own.mime, &own.data)), None));
    }
    let mut light = None;
    let mut dark = None;
    for image in branding_image::Entity::find().all(db).await? {
        match image.kind.as_str() {
            "logo" => light = Some(url(&image.mime, &image.data)),
            "logo-dark" => dark = Some(url(&image.mime, &image.data)),
            _ => {}
        }
    }
    Ok(match (light, dark) {
        (None, dark) => (dark, None),
        both => both,
    })
}

/// A logo's type from its bytes: PNG, JPEG or WebP (`None`: another).
pub fn raster_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// A request's host without its port, lowercase.
pub fn bare_host(host: &str) -> String {
    host.rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map_or(host, |(h, _)| h)
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

/// Whether `host` (a request's, its port dropped) is one of `domains`
/// (exact, or a wildcard's single label).
pub fn serves(host: &str, domains: &[String]) -> bool {
    let host = bare_host(host);
    !host.is_empty()
        && domains.iter().any(|d| {
            let d = d.to_ascii_lowercase();
            match d.strip_prefix("*.") {
                Some(base) => host
                    .strip_suffix(base)
                    .and_then(|label| label.strip_suffix('.'))
                    .is_some_and(|label| !label.is_empty() && !label.contains('.')),
                None => host == d,
            }
        })
}

/// The exact addresses of a bypass list (`/32`, `/128`), bare: the ones
/// Traefik can match in `X-Forwarded-For`.
pub fn exact_addresses(bypass: &[String]) -> Vec<String> {
    bypass
        .iter()
        .filter_map(|c| {
            let (address, prefix) = c.split_once('/')?;
            let full = if address.contains(':') { "128" } else { "32" };
            (prefix == full).then(|| address.to_owned())
        })
        .collect()
}

/// A Go regular expression matching an `X-Forwarded-For` whose last entry
/// (the one the trusted proxy wrote) is one of `addresses`.
pub fn forwarded_for(addresses: &[String]) -> String {
    let any = addresses
        .iter()
        .map(|a| regex::escape(a))
        .collect::<Vec<_>>()
        .join("|");
    format!(r"(^|,\s*)({any})$")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn facts() -> Facts {
        Facts {
            title: "Back <soon>".into(),
            subtitle: String::new(),
            eta: Some((Utc.with_ymd_and_hms(2026, 10, 22, 7, 30, 0).unwrap(), 420)),
            contact: Some("ops@shop.vn".into()),
            logo: Some("data:image/png;base64,AAAA".into()),
            logo_dark: None,
            service: "web".into(),
            project: "shop".into(),
            lang: Lang::En,
        }
    }

    #[test]
    fn placeholders_are_filled_and_escaped() {
        let out = render(
            "<h1>{{title}}</h1><p>{{ subtitle }}</p><i>{{eta}}</i><b>{{nope}}</b>{{contact_href}}|{{contact}}<img src=\"{{logo}}\">{{",
            &facts(),
        )
        .unwrap();
        assert_eq!(
            out,
            "<h1>Back &lt;soon&gt;</h1><p>We will be back soon.</p><i>14:30, 22/10/2026 (GMT+7)</i><b>{{nope}}</b>|ops@shop.vn<img src=\"data:image/png;base64,AAAA\">{{"
        );
    }

    #[test]
    fn pages_stay_bounded() {
        let mut big = facts();
        big.logo = Some(format!("data:image/png;base64,{}", "A".repeat(200_000)));
        assert!(render(&"{{logo}}".repeat(5), &big).is_none(), "five logos");
        assert!(render(&"{{logo}}".repeat(4), &big).is_some());
        assert!(
            render(&"{{title}}".repeat(200_000), &big).is_none(),
            "grows past 1 MB"
        );
        // Then the built-in page, its large logo left out.
        let page = page(Some(&"{{logo}}".repeat(5)), &big);
        assert!(page.contains("Back &lt;soon&gt;") && !page.contains("AAAA"));
        assert!(page.len() < 4096);
    }

    #[test]
    fn the_built_in_page() {
        let page = builtin(&facts());
        assert!(page.contains("<h1>Back &lt;soon&gt;</h1>"));
        assert!(page.contains("Back at 14:30, 22/10/2026 (GMT+7) · ops@shop.vn"));
        assert!(page.contains(r#"<img src="data:image/png;base64,AAAA" alt="">"#));
        let mut link = facts();
        link.contact = Some("https://status.shop.vn".into());
        link.eta = None;
        assert!(builtin(&link).contains(
            r#"<p class="meta"><a href="https://status.shop.vn">https://status.shop.vn</a></p>"#
        ));
        let bare = Facts::default();
        let page = builtin(&bare);
        assert!(
            page.contains("<h1>Under maintenance</h1>")
                && !page.contains("meta\"")
                && !page.contains("<img")
        );
        let vi = Facts {
            lang: Lang::Vi,
            ..facts()
        };
        assert!(builtin(&vi).contains("Dự kiến trở lại lúc 14:30"));
        assert!(
            builtin(&Facts {
                lang: Lang::Vi,
                ..Facts::default()
            })
            .contains("<h1>Đang bảo trì</h1>")
        );
    }

    #[test]
    fn times() {
        let at = Utc.with_ymd_and_hms(2026, 10, 22, 7, 30, 0).unwrap();
        assert_eq!(eta_text(at, 0), "07:30, 22/10/2026 (GMT)");
        assert_eq!(eta_text(at, -150), "05:00, 22/10/2026 (GMT-2:30)");
        let now = at - chrono::Duration::minutes(10);
        assert_eq!(retry_after(Some(at), now), 600);
        assert_eq!(retry_after(Some(at), at + chrono::Duration::hours(1)), 60);
        assert_eq!(
            retry_after(Some(at + chrono::Duration::days(9)), now),
            86_400
        );
        assert_eq!(retry_after(None, now), 300);
    }

    #[test]
    fn contacts() {
        for ok in ["ops@shop.vn", "https://status.shop.vn/x?y=1"] {
            assert!(valid_contact(ok), "{ok}");
        }
        for bad in [
            "ops",
            "a b@c.d",
            "javascript:alert(1)",
            "http://x.y",
            "x@y",
            "\"@x.y",
            "https://x\"y",
        ] {
            assert!(!valid_contact(bad), "{bad}");
        }
        assert_eq!(contact_href(Some("ops@shop.vn")), "");
    }

    #[test]
    fn hosts() {
        let domains = vec!["shop.vn".to_owned(), "*.apps.vn".to_owned()];
        for ok in ["shop.vn", "SHOP.vn:443", "shop.vn.", "a.apps.vn"] {
            assert!(serves(ok, &domains), "{ok}");
        }
        for bad in [
            "panel.vn",
            "apps.vn",
            "a.b.apps.vn",
            "evil-shop.vn",
            "",
            "103.90.224.55:3000",
        ] {
            assert!(!serves(bad, &domains), "{bad}");
        }
    }

    #[test]
    fn forwarded_matches_only_the_last_entry() {
        let exact = exact_addresses(&[
            "10.8.0.0/24".into(),
            "1.2.3.4/32".into(),
            "2001:db8::1/128".into(),
        ]);
        assert_eq!(exact, ["1.2.3.4", "2001:db8::1"]);
        let re = regex::Regex::new(&forwarded_for(&exact)).unwrap();
        for ok in [
            "1.2.3.4",
            "9.9.9.9, 1.2.3.4",
            "9.9.9.9,1.2.3.4",
            "2001:db8::1",
        ] {
            assert!(re.is_match(ok), "{ok}");
        }
        for bad in [
            "1.2.3.45",
            "11.2.3.4",
            "1.2.3.4, 9.9.9.9",
            "1x2x3x4",
            "2001:db8::10",
        ] {
            assert!(!re.is_match(bad), "{bad}");
        }
    }

    #[test]
    fn the_starter_renders() {
        let out = render(&starter(), &facts()).unwrap();
        assert!(
            out.contains("<h1>Back &lt;soon&gt;</h1>")
                && out.contains("14:30, 22/10/2026 (GMT+7) · ops@shop.vn")
        );
        assert!(!out.contains("{{"));
    }

    #[test]
    fn rasters() {
        assert_eq!(raster_mime(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(raster_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(raster_mime(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(raster_mime(b"<svg xmlns="), None);
    }
}
