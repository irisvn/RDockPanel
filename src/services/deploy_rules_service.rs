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
}

impl Default for PanelConfig {
    fn default() -> Self {
        Self {
            bind_roots: vec![],
            reserved_ports: vec![],
            cert_resolver: String::new(),
            https_port: 443,
            traefik_dir: PathBuf::from("storage/traefik/dynamic"),
            traefik_service: "rdp-traefik".to_owned(),
            service_domain: String::new(),
            public_ip: String::new(),
            templates_url:
                "https://raw.githubusercontent.com/easypanel-io/templates/main/templates".to_owned(),
        }
    }
}

impl PanelConfig {
    /// From the app's configuration; defaults when the section is missing.
    pub fn from_config(config: &rustcms::Config) -> Self {
        config.section::<Self>("panel").unwrap_or_default()
    }
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
    host.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|label| {
            let b = label.as_bytes();
            (1..=63).contains(&b.len())
                && b[0].is_ascii_alphanumeric()
                && b[b.len() - 1].is_ascii_alphanumeric()
                && b.iter()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        })
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
