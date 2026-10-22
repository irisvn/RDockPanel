//! What projects and services may be named and configured with: names that
//! become Docker object names, hostnames and paths that become Traefik
//! rules, ports, images, and host directories a service may mount.
//!
//! Pure checks (no database, no Docker), shared by the admin resources and,
//! later, the deployment job. [`PanelConfig`] is the `[panel]` section of the
//! configuration (`config/panel.toml`).

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

/// The `[panel]` configuration section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PanelConfig {
    /// Host directories services may bind-mount (system managers only).
    pub bind_roots: Vec<PathBuf>,
    /// Ports services may not publish.
    pub reserved_ports: Vec<u16>,
    /// Traefik's certificate resolver for HTTPS domains (`""`: Traefik's
    /// default certificate, as in development).
    pub cert_resolver: String,
    /// The public HTTPS port HTTP routes redirect to (443; 8443 with
    /// `scripts/dev-swarm.sh`).
    pub https_port: u16,
    /// Traefik's file provider directory: the panel writes the routes there.
    pub traefik_dir: PathBuf,
    /// The same directory as Traefik sees it (uploaded certificates' files
    /// are named so in its configuration).
    pub traefik_mount: String,
    /// Traefik's Swarm service, attached to each project's network (empty:
    /// Traefik is not a Swarm service the panel manages).
    pub traefik_service: String,
    /// The domain generated ones live under (`<project>-<service>.<this>`,
    /// a wildcard DNS record to the server); empty: none generated.
    pub service_domain: String,
    /// The server's public address (`$(EASYPANEL_HOST_IP)`); empty: unknown.
    pub public_ip: String,
    /// Where EasyPanel's templates are fetched from (their repository's
    /// `templates/` folder, raw files).
    pub templates_url: String,
    /// Compose services' files (one directory each). The Docker daemon
    /// reads relative paths of a Compose file there: the same path on the
    /// host and in the panel (the installer: /var/lib/rdockpanel/compose).
    pub compose_dir: PathBuf,
    /// The panel as Traefik reaches it (`http://rdockpanel:3000` installed):
    /// a service in maintenance is routed to its page there. Empty: no
    /// maintenance mode.
    pub internal_url: String,
    /// The panel's own host (from `APP_URL`): no service may route it.
    #[serde(skip)]
    pub panel_host: String,
    /// The panel's address (`APP_URL`), as its users open it (no
    /// credentials, query or fragment).
    #[serde(skip)]
    pub panel_url: String,
    /// Installed by install.sh (it sets RUSTCMS_PANEL__INSTALLED); else run
    /// from source.
    pub installed: bool,
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            bind_roots: vec![],
            reserved_ports: vec![],
            cert_resolver: String::new(),
            https_port: 443,
            traefik_dir: PathBuf::from("storage/traefik/dynamic"),
            traefik_mount: "/etc/traefik/dynamic".to_owned(),
            traefik_service: "rdp-traefik".to_owned(),
            service_domain: String::new(),
            public_ip: String::new(),
            templates_url:
                "https://raw.githubusercontent.com/easypanel-io/templates/main/templates".to_owned(),
            compose_dir: PathBuf::from("storage/compose"),
            internal_url: String::new(),
            panel_host: String::new(),
            panel_url: String::new(),
            installed: false,
        }
    }
}

impl PanelConfig {
    /// From the app's configuration; defaults for what the section leaves
    /// out (or for all of it, missing). A value of the wrong kind is an
    /// error: never read as a default (`installed`, `reserved_ports`…).
    pub fn from_config(config: &rustcms::Config) -> Result<Self, rustcms::config::ConfigError> {
        let mut panel = config.section::<Self>("panel")?;
        panel.panel_host = panel_host(&config.app.url);
        panel.panel_url = shown_url(&config.app.url);
        Ok(panel)
    }
}

/// The host of `url` (`https://panel.example.com:3000/` → `panel.example.com`),
/// lowercase; empty for none.
fn panel_host(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    host.split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// `url` as it may be shown: without credentials, query or fragment, no
/// trailing slash (empty when it is no URL).
fn shown_url(url: &str) -> String {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return String::new();
    };
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed.as_str().trim_end_matches('/').to_owned()
}

/// The longest command a service runs (characters, as its column).
pub const MAX_COMMAND: usize = 1000;
/// The longest path inside a repository (characters, as its columns).
pub const MAX_REPO_PATH: usize = 255;
/// The longest redirect replacement (characters, as its column).
pub const MAX_REPLACEMENT: usize = 500;

/// A domain's host as stored and checked: trimmed, lowercase
/// (` Shop.Example.COM` → `shop.example.com`).
pub fn normalize_host(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

/// The most a sealed value holds (its JSON): RustCMS's limit. A service's
/// environment is one value, and so are its files together (a deployment
/// keeps them so).
pub const SEALED_MAX: usize = rustcms::crypto::MAX_PLAINTEXT;

/// Whether `value` can be sealed (its JSON at most [`SEALED_MAX`] bytes).
pub fn sealable<T: serde::Serialize>(value: &T) -> bool {
    serde_json::to_vec(value).is_ok_and(|json| json.len() <= SEALED_MAX)
}

/// Whether `host` is the panel's own (no service may route it).
pub fn is_panel_host(host: &str) -> bool {
    let panel = &config().panel_host;
    !panel.is_empty() && panel != "localhost" && host.eq_ignore_ascii_case(panel)
}

static CONFIG: std::sync::OnceLock<PanelConfig> = std::sync::OnceLock::new();

/// Sets the configuration the checks use (at boot; the first call wins).
pub fn configure(config: PanelConfig) {
    let _ = CONFIG.set(config);
}

/// The configuration the checks use (defaults before [`configure`]).
pub fn config() -> &'static PanelConfig {
    CONFIG.get_or_init(PanelConfig::default)
}

/// Project and service names: they become `rdp_<project>`, `<project>_<service>`
/// and DNS names inside the project's network. 1 to 40 lowercase letters,
/// digits and `-`, starting with a letter, not ending with `-`.
pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=40).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && !name.ends_with('-')
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

/// A volume's own name (the Docker volume is `rdp_<project>_<service>_<name>`).
pub fn valid_volume_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=40).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

/// A hostname Traefik routes (lowercase): labels of letters, digits and `-`,
/// at least two labels (`app.example.com`, `app.localhost`), 253 bytes at most.
pub fn valid_hostname(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    host.len() <= 253 && labels.len() >= 2 && labels.iter().all(|label| valid_label(label))
}

/// A domain's host: a hostname, or a wildcard `*.example.com` (every name
/// one level under it).
pub fn valid_domain_host(host: &str) -> bool {
    match host.strip_prefix("*.") {
        Some(rest) => valid_hostname(rest),
        None => valid_hostname(host),
    }
}

/// A Traefik middleware a domain names (`ratelimit@file`, `auth@docker`):
/// letters, digits, `-`, `_`, `.`, one `@`.
pub fn valid_middleware(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name.matches('@').count() <= 1
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
}

fn valid_label(label: &str) -> bool {
    let b = label.as_bytes();
    (1..=63).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b[b.len() - 1].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// A path prefix Traefik routes: `/` or `/segment/...` of URL-safe characters
/// (nothing that could close a rule: no quotes, backticks, spaces).
pub fn valid_route_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 255
        && !path.contains("//")
        && !path.split('/').any(|segment| segment == "..")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~'))
}

pub fn valid_port(port: i64) -> bool {
    (1..=65535).contains(&port)
}

/// `[registry[:port]/]name[:tag|@digest]`, loosely: Docker has the final say.
pub fn valid_image_reference(image: &str) -> bool {
    !image.is_empty()
        && image.len() <= 255
        && image
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | ':' | '.' | '_' | '-' | '@'))
        && image
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric())
}

/// A path inside the container: absolute, normalized, not `/`.
pub fn valid_container_path(path: &str) -> bool {
    let p = Path::new(path);
    path.len() <= 512
        && p.is_absolute()
        && path != "/"
        && !path.ends_with('/')
        && p.components().all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
        // `components()` skips `.`: check the text too.
        && !path.split('/').skip(1).any(|segment| matches!(segment, "" | "." | ".."))
}

/// Never mountable, whatever the roots: the host itself, its kernel views,
/// Docker and the panel.
const FORBIDDEN: [&str; 9] = [
    "/proc",
    "/sys",
    "/dev",
    "/boot",
    "/var/lib/docker",
    "/var/run/docker.sock",
    "/run/docker.sock",
    "/etc/rdockpanel",
    "/run/secrets",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindRefusal {
    NotAllowed,
    NotAbsolute,
    Missing,
    OutsideRoots,
    Forbidden,
}

impl BindRefusal {
    pub fn message(&self) -> &'static str {
        match self {
            Self::NotAllowed => "bind mounts are not enabled on this server",
            Self::NotAbsolute => "must be an absolute host path",
            Self::Missing => "does not exist on the server",
            Self::OutsideRoots => "is outside the directories allowed for bind mounts",
            Self::Forbidden => "may not be mounted",
        }
    }
}

/// The host directory a bind mount may use, resolved (symlinks followed) and
/// checked to stay under one of `roots`, outside [`FORBIDDEN`] and not above
/// any of it (mounting `/run` would expose the Docker socket).
///
/// This checks the path as it is now. Docker resolves it again when the task
/// starts, so a directory replaced by a symlink in between could still point
/// elsewhere: deployments check it again, and the roots must be directories
/// that no service and no untrusted user can write.
pub fn bind_source(source: &str, roots: &[PathBuf]) -> Result<PathBuf, BindRefusal> {
    if roots.is_empty() {
        return Err(BindRefusal::NotAllowed);
    }
    let path = Path::new(source);
    if !path.is_absolute() || source.contains('\0') {
        return Err(BindRefusal::NotAbsolute);
    }
    // Inside a forbidden place, or above one (its parent exposes it).
    let forbidden = |p: &Path| {
        p == Path::new("/")
            || FORBIDDEN
                .iter()
                .any(|f| p.starts_with(f) || Path::new(f).starts_with(p))
    };
    // As written, then as resolved (a symlink into a forbidden place).
    if forbidden(path) {
        return Err(BindRefusal::Forbidden);
    }
    let resolved = std::fs::canonicalize(path).map_err(|_| BindRefusal::Missing)?;
    if forbidden(&resolved) {
        return Err(BindRefusal::Forbidden);
    }
    let inside = roots.iter().any(|root| {
        std::fs::canonicalize(root)
            .is_ok_and(|root| root != Path::new("/") && resolved.starts_with(&root))
    });
    if inside {
        Ok(resolved)
    } else {
        Err(BindRefusal::OutsideRoots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel_config(path: &str, value: serde_json::Value) -> Result<PanelConfig, String> {
        let config = rustcms_testing::test_config("mysql://unused")
            .with(path, value)
            .unwrap();
        PanelConfig::from_config(&config).map_err(|e| e.to_string())
    }

    #[test]
    fn a_malformed_panel_section_is_an_error() {
        // Left out: the defaults.
        let panel = panel_config("panel.public_ip", serde_json::json!("203.0.113.7")).unwrap();
        assert_eq!(panel.public_ip, "203.0.113.7");
        assert_eq!(panel.https_port, 443);
        // Of the wrong kind: never the defaults.
        for (path, value) in [
            ("panel.installed", serde_json::json!("yes")),
            ("panel.https_port", serde_json::json!("oops")),
            ("panel.reserved_ports", serde_json::json!("22,80")),
        ] {
            let err = panel_config(path, value).unwrap_err();
            assert!(err.contains(path.trim_start_matches("panel.")), "{err}");
        }
    }

    #[test]
    fn the_panels_url_as_shown() {
        assert_eq!(shown_url("http://localhost:8000"), "http://localhost:8000");
        assert_eq!(
            shown_url("https://user:secret@panel.example.com/admin/?token=x#y"),
            "https://panel.example.com/admin"
        );
        assert_eq!(shown_url("not a url"), "");
    }

    #[test]
    fn the_panels_host() {
        assert_eq!(panel_host("https://Panel.Example.com"), "panel.example.com");
        assert_eq!(panel_host("http://1.2.3.4:3000/admin?x=1"), "1.2.3.4");
        assert_eq!(
            panel_host("https://u:p@panel.example.com:8443/"),
            "panel.example.com"
        );
        assert_eq!(panel_host(""), "");
    }

    #[test]
    fn names() {
        for ok in ["web", "my-app2", "a"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "",
            "Web",
            "2web",
            "web-",
            "web_app",
            "web.app",
            &"a".repeat(41),
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert!(
            valid_volume_name("data_1") && !valid_volume_name("../x") && !valid_volume_name("_x")
        );
    }

    #[test]
    fn hostnames_and_paths() {
        for ok in ["app.example.com", "whoami.localhost", "a-b.c1.io"] {
            assert!(valid_hostname(ok), "{ok}");
        }
        for bad in [
            "localhost",
            "App.example.com",
            "-a.com",
            "a..com",
            "a.com`",
            "a b.com",
            "*.a.com",
        ] {
            assert!(!valid_hostname(bad), "{bad}");
        }
        for ok in ["/", "/api", "/api/v1", "/a-b_c.d~"] {
            assert!(valid_route_path(ok), "{ok}");
        }
        for bad in ["", "api", "/a`)", "/a b", "//x", "/a/../b", "/a?x"] {
            assert!(!valid_route_path(bad), "{bad}");
        }
    }

    #[test]
    fn container_paths_and_images() {
        assert!(valid_container_path("/data") && valid_container_path("/var/lib/app"));
        for bad in ["/", "data", "/data/", "/a/../b", "/a/./b", "//a"] {
            assert!(!valid_container_path(bad), "{bad}");
        }
        assert!(
            valid_image_reference("ghcr.io/org/app:1.2@sha256:abc") && !valid_image_reference("-x")
        );
    }

    #[test]
    fn bind_sources_stay_under_roots() {
        let base = std::env::temp_dir().join(format!("rdp-binds-{}", std::process::id()));
        let root = base.join("root");
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::create_dir_all(base.join("outside")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(base.join("outside"), root.join("escape")).unwrap();
        let roots = vec![root.clone()];
        let s = |p: &Path| p.to_str().unwrap().to_owned();

        assert_eq!(
            bind_source(&s(&root.join("app")), &[]),
            Err(BindRefusal::NotAllowed)
        );
        assert_eq!(
            bind_source(&s(&root.join("app")), &roots),
            Ok(root.join("app").canonicalize().unwrap())
        );
        assert_eq!(bind_source("app", &roots), Err(BindRefusal::NotAbsolute));
        assert_eq!(
            bind_source(&s(&root.join("nope")), &roots),
            Err(BindRefusal::Missing)
        );
        assert_eq!(
            bind_source(&s(&base.join("outside")), &roots),
            Err(BindRefusal::OutsideRoots)
        );
        #[cfg(unix)]
        assert_eq!(
            bind_source(&s(&root.join("escape")), &roots),
            Err(BindRefusal::OutsideRoots)
        );
        assert_eq!(
            bind_source(&s(&root.join("app/../../outside")), &roots),
            Err(BindRefusal::OutsideRoots)
        );
        assert_eq!(
            bind_source("/", &[PathBuf::from("/")]),
            Err(BindRefusal::Forbidden)
        );
        assert_eq!(
            bind_source("/proc/self", &[PathBuf::from("/")]),
            Err(BindRefusal::Forbidden)
        );
        // Above a forbidden place: its children would be exposed.
        for parent in ["/run", "/var/run", "/etc", "/var", "/var/lib"] {
            assert_eq!(
                bind_source(parent, &[PathBuf::from("/")]),
                Err(BindRefusal::Forbidden),
                "{parent}"
            );
        }

        std::fs::remove_dir_all(base).unwrap();
    }
}
