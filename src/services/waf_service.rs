//! The WAF (Settings → Security): CrowdSec's AppSec through its Traefik
//! plugin, in front of the services' domains.
//!
//! - **Settings**: the panel's mode (`off`, `detect`: recorded, never blocked;
//!   `block`), level (`basic`: known exploits and secret files; `strict`:
//!   with the OWASP Core Rule Set), community sharing, allowlisted addresses.
//!   A service follows them, or sets `on`/`off` for itself (`services.waf`).
//! - **Routes**: a service whose WAF is on names `rdp-waf-buffer@file` then
//!   `rdp-waf@file` first on every router reaching its app (the buffer keeps
//!   bodies whole: the plugin alone truncates bodies over ~1 MB).
//! - **Definitions** (`rdp-waf.yml`, beside the routes): the plugin while it
//!   works; valid no-op middlewares when it does not (Traefik disables a
//!   router whose middleware does not exist: the sites would answer 404).
//! - **Fail-open**: an unreachable or failing WAF lets requests through; the
//!   panel says so. Never fail-closed.
//! - The `rdp-waf` prefix is reserved: a domain cannot name these middlewares.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Middlewares beginning with this are the panel's (refused on a domain).
pub const PREFIX: &str = "rdp-waf";
/// The WAF a router names, and the buffer before it.
pub const MIDDLEWARE: &str = "rdp-waf@file";
pub const BUFFER: &str = "rdp-waf-buffer@file";
/// The canary's own WAF (no allowlist: it must always be inspected).
pub const CANARY: &str = "rdp-waf-canary@file";
/// The definitions' file, beside the routes (not a route file: never pruned).
pub const FILE: &str = "rdp-waf.yml";
/// Where the settings are kept (`panel_settings`).
pub const SETTINGS_KEY: &str = "waf.settings";
/// The plugin's name in Traefik's static configuration, and its module.
pub const PLUGIN: &str = "rdpwaf";
pub const MODULE: &str = "github.com/maxlerebourg/crowdsec-bouncer-traefik-plugin";
/// The CrowdSec service, its AppSec listeners (block, detect) and network.
pub const CROWDSEC: &str = "rdp-crowdsec";
pub const NETWORK: &str = "rdp-crowdsec";
pub const APPSEC_BLOCK_PORT: u16 = 7422;
pub const APPSEC_DETECT_PORT: u16 = 7423;
/// Where Traefik reads the bouncer's key (a Docker secret).
pub const KEY_FILE: &str = "/run/secrets/rdp_crowdsec_bouncer";
/// Bodies buffered before the WAF, in memory (larger ones go to a file).
const BUFFER_MEMORY: u64 = 10 * 1024 * 1024;
/// How long the plugin waits for AppSec before letting a request through.
const APPSEC_TIMEOUT_SECONDS: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Off,
    Detect,
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    #[default]
    Basic,
    Strict,
}

/// The panel's WAF settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub level: Level,
    /// Addresses (IPs or CIDRs) never inspected: trusted, not checked.
    #[serde(default)]
    pub allowlist: Vec<String>,
    /// Share attack signals with CrowdSec's community (off by default).
    #[serde(default)]
    pub sharing: bool,
}

impl Settings {
    /// As saved; none saved: off. Unreadable: off (said in the log).
    pub async fn load<C: sea_orm::ConnectionTrait>(db: &C) -> Result<Self, sea_orm::DbErr> {
        let Some(text) = crate::models::panel_setting::get(db, SETTINGS_KEY).await? else {
            return Ok(Self::default());
        };
        Ok(serde_json::from_str(&text).unwrap_or_else(|err| {
            tracing::warn!(%err, "the WAF settings cannot be read: off");
            Self::default()
        }))
    }

    pub async fn store<C: sea_orm::ConnectionTrait>(&self, db: &C) -> Result<(), sea_orm::DbErr> {
        let text = serde_json::to_string(self).unwrap_or_default();
        crate::models::panel_setting::set(db, SETTINGS_KEY, &text).await
    }

    /// Its allowlist checked: IPs or CIDRs only.
    pub fn problems(&self) -> Vec<String> {
        self.allowlist
            .iter()
            .filter(|a| !valid_address(a))
            .map(|a| format!("{a} is not an IP address or a CIDR range"))
            .collect()
    }
}

/// An IP address or a CIDR range (`203.0.113.7`, `2001:db8::/32`).
pub fn valid_address(text: &str) -> bool {
    let (ip, bits) = match text.split_once('/') {
        Some((ip, bits)) => (ip, Some(bits)),
        None => (text, None),
    };
    let Ok(ip) = ip.parse::<std::net::IpAddr>() else {
        return false;
    };
    match bits {
        None => true,
        Some(bits) => bits
            .parse::<u8>()
            .is_ok_and(|b| b <= if ip.is_ipv4() { 32 } else { 128 }),
    }
}

/// A service's own setting (`services.waf`): none, `on`, `off`.
pub fn valid_service_setting(value: Option<&str>) -> bool {
    matches!(value, None | Some("on" | "off"))
}

/// Whether a service's domains go through the WAF: the panel's mode on, and
/// the service not turned off.
pub fn effective(settings: &Settings, service: Option<&str>) -> bool {
    settings.mode != Mode::Off && service != Some("off")
}

/// How protective a service's setting is (higher: more), for the panel's mode.
fn strength(settings: &Settings, service: Option<&str>) -> u8 {
    if !effective(settings, service) {
        0
    } else {
        match settings.mode {
            Mode::Off => 0,
            Mode::Detect => 1,
            Mode::Block => 2,
        }
    }
}

/// Whether changing a service's setting from `before` to `after` lowers its
/// protection (only the panel's administrators may).
pub fn service_change_lowers(
    settings: &Settings,
    before: Option<&str>,
    after: Option<&str>,
) -> bool {
    after == Some("off") && before != Some("off")
        || strength(settings, after) < strength(settings, before)
}

/// The middlewares of `rdp-waf.yml`: the buffer always; the WAF and the
/// canary's as the plugin when `healthy`, valid no-ops otherwise (and when
/// off). `trusted`: the proxies whose `X-Forwarded-For` names the visitor.
pub fn definitions(settings: &Settings, healthy: bool, trusted: &[String]) -> Value {
    let mut middlewares = serde_json::Map::new();
    middlewares.insert(
        "rdp-waf-buffer".into(),
        json!({"buffering": {"memRequestBodyBytes": BUFFER_MEMORY, "maxRequestBodyBytes": 0}}),
    );
    let plugin = |allowlist: &[String], port: u16| {
        json!({"plugin": {PLUGIN: {
            "enabled": true,
            "logLevel": "WARN",
            "crowdsecMode": "appsec",
            "crowdsecAppsecEnabled": true,
            "crowdsecAppsecHost": format!("{CROWDSEC}:{port}"),
            "crowdsecLapiKeyFile": KEY_FILE,
            "crowdsecAppsecUnreachableBlock": false,
            "crowdsecAppsecFailureBlock": false,
            "httpTimeoutSeconds": APPSEC_TIMEOUT_SECONDS,
            "forwardedHeadersTrustedIPs": trusted,
            "clientTrustedIPs": allowlist,
        }}})
    };
    // A real middleware that changes nothing a visitor sees (`headers: {}`
    // alone is invalid: Traefik would drop the whole file).
    let noop = json!({"headers": {"customRequestHeaders": {"X-Rdp-Waf": "noop"}}});
    let on = healthy && settings.mode != Mode::Off;
    middlewares.insert(
        "rdp-waf".into(),
        if on {
            let port = if settings.mode == Mode::Detect {
                APPSEC_DETECT_PORT
            } else {
                APPSEC_BLOCK_PORT
            };
            plugin(&settings.allowlist, port)
        } else {
            noop.clone()
        },
    );
    // The canary is the plugin whenever the WAF is on, healthy or not: its
    // router alone answers 404 while the plugin is not loaded (the sites
    // keep their no-ops), and an attack answered 403 there proves the
    // plugin, the key and AppSec all work. It asks the blocking listener,
    // in Detect mode too.
    middlewares.insert(
        "rdp-waf-canary".into(),
        if settings.mode != Mode::Off {
            plugin(&[], APPSEC_BLOCK_PORT)
        } else {
            noop
        },
    );
    json!({"http": {"middlewares": middlewares}})
}

/// The middlewares a router reaching an app gets first, when its WAF is on.
pub fn chain() -> [&'static str; 2] {
    [BUFFER, MIDDLEWARE]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(mode: Mode) -> Settings {
        Settings {
            mode,
            ..Settings::default()
        }
    }

    #[test]
    fn a_service_follows_the_panel_unless_turned_off() {
        assert!(!effective(&with(Mode::Off), None));
        assert!(!effective(&with(Mode::Off), Some("on")));
        assert!(effective(&with(Mode::Detect), None));
        assert!(effective(&with(Mode::Block), Some("on")));
        assert!(!effective(&with(Mode::Block), Some("off")));
    }

    #[test]
    fn turning_a_service_off_lowers_it_turning_it_on_does_not() {
        let block = with(Mode::Block);
        assert!(service_change_lowers(&block, None, Some("off")));
        assert!(service_change_lowers(&block, Some("on"), Some("off")));
        assert!(!service_change_lowers(&block, Some("off"), None));
        assert!(!service_change_lowers(&block, Some("off"), Some("on")));
        assert!(!service_change_lowers(&block, None, Some("on")));
        // Even while the panel is off, `off` is a lowering of what will be.
        assert!(service_change_lowers(&with(Mode::Off), None, Some("off")));
    }

    #[test]
    fn a_failing_plugin_becomes_a_valid_no_op() {
        let block = with(Mode::Block);
        let trusted = vec!["173.245.48.0/20".to_owned()];
        let healthy = definitions(&block, true, &trusted);
        let plugin = &healthy["http"]["middlewares"]["rdp-waf"]["plugin"][PLUGIN];
        assert_eq!(plugin["crowdsecAppsecUnreachableBlock"], false);
        assert_eq!(plugin["crowdsecAppsecFailureBlock"], false);
        assert_eq!(plugin["forwardedHeadersTrustedIPs"], json!(trusted));
        let broken = definitions(&block, false, &trusted);
        let noop = &broken["http"]["middlewares"]["rdp-waf"];
        assert!(
            noop["headers"]["customRequestHeaders"].is_object(),
            "{noop}"
        );
        assert!(broken["http"]["middlewares"]["rdp-waf-buffer"]["buffering"].is_object());
        // The canary stays the plugin: it proves the plugin loads again.
        assert!(broken["http"]["middlewares"]["rdp-waf-canary"]["plugin"].is_object());
        let off = definitions(&with(Mode::Off), false, &trusted);
        assert!(off["http"]["middlewares"]["rdp-waf-canary"]["headers"].is_object());
    }

    #[test]
    fn detect_and_block_reach_their_own_listener_and_the_canary_ignores_the_allowlist() {
        let mut detect = with(Mode::Detect);
        detect.allowlist = vec!["203.0.113.7".to_owned()];
        let d = definitions(&detect, true, &[]);
        assert_eq!(
            d["http"]["middlewares"]["rdp-waf"]["plugin"][PLUGIN]["crowdsecAppsecHost"],
            "rdp-crowdsec:7423"
        );
        assert_eq!(
            d["http"]["middlewares"]["rdp-waf"]["plugin"][PLUGIN]["clientTrustedIPs"],
            json!(["203.0.113.7"])
        );
        assert_eq!(
            d["http"]["middlewares"]["rdp-waf-canary"]["plugin"][PLUGIN]["clientTrustedIPs"],
            json!([])
        );
        assert_eq!(
            d["http"]["middlewares"]["rdp-waf-canary"]["plugin"][PLUGIN]["crowdsecAppsecHost"],
            "rdp-crowdsec:7422"
        );
        let b = definitions(&with(Mode::Block), true, &[]);
        assert_eq!(
            b["http"]["middlewares"]["rdp-waf"]["plugin"][PLUGIN]["crowdsecAppsecHost"],
            "rdp-crowdsec:7422"
        );
    }

    #[test]
    fn addresses_are_ips_or_cidrs() {
        assert!(valid_address("203.0.113.7"));
        assert!(valid_address("2001:db8::/32"));
        assert!(valid_address("10.0.0.0/8"));
        assert!(!valid_address("10.0.0.0/33"));
        assert!(!valid_address("example.com"));
        assert!(!valid_address(""));
    }
}
