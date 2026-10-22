//! A service's rate limit (its Security tab): Traefik's `rateLimit` on the
//! routers serving its domains, counted by visitor.
//!
//! Who the visitor is depends on how the request came:
//! - straight to Traefik (80/443, published on the host): the peer;
//! - through a Cloudflare edge (a proxied DNS record) or the tunnel's
//!   cloudflared: the last `X-Forwarded-For` entry, which Traefik keeps only
//!   from those peers (`forwardedHeaders.trustedIPs`: Cloudflare's ranges on
//!   `web`/`websecure`, the tunnel's network on `tunnel`). Anyone else's is
//!   dropped, so nobody picks their own key.
//!
//! IPv6 visitors count by their /64. Exempt addresses match the peer only:
//! visitors reaching Traefik directly (a VPN, a monitor), never Cloudflare's.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::services::firewall_rules::Cidr;

/// Cloudflare's edge addresses (cloudflare.com/ips-v4 and ips-v6, 2026-10-22).
/// A range missing here: its visitors count by the edge's address.
pub const CLOUDFLARE: &[&str] = &[
    "173.245.48.0/20",
    "103.21.244.0/22",
    "103.22.200.0/22",
    "103.31.4.0/22",
    "141.101.64.0/18",
    "108.162.192.0/18",
    "190.93.240.0/20",
    "188.114.96.0/20",
    "197.234.240.0/22",
    "198.41.128.0/17",
    "162.158.0.0/15",
    "104.16.0.0/13",
    "104.24.0.0/14",
    "172.64.0.0/13",
    "131.0.72.0/22",
    "2400:cb00::/32",
    "2606:4700::/32",
    "2803:f800::/32",
    "2405:b500::/32",
    "2405:8100::/32",
    "2a06:98c0::/29",
    "2c0f:f248::/32",
];

/// The periods offered (Traefik's durations).
pub const PERIODS: &[&str] = &["1s", "1m", "1h"];
/// Bounds of the average and the burst.
pub const MAX: u32 = 100_000;
/// Exempt addresses a service may have.
pub const MAX_EXEMPT: usize = 50;

/// A service's limit: `average` requests per `period` by visitor, `burst`
/// at once; `exempt` (canonical networks) not limited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimit {
    pub average: u32,
    pub period: String,
    pub burst: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exempt: Vec<String>,
}

impl RateLimit {
    /// The limit of a service row (`None`: off, or not a valid one).
    pub fn of(row: &crate::models::service::Model) -> Option<RateLimit> {
        let average = u32::try_from(row.rate_limit_average?).ok()?;
        let burst = u32::try_from(row.rate_limit_burst.unwrap_or(1)).ok()?;
        let period = row.rate_limit_period.clone().unwrap_or_else(|| "1s".into());
        if !(1..=MAX).contains(&average) || !(1..=MAX).contains(&burst) {
            return None;
        }
        if !PERIODS.contains(&period.as_str()) {
            return None;
        }
        Some(RateLimit {
            average,
            period,
            burst,
            exempt: exempt(row.rate_limit_exempt.as_deref()).unwrap_or_default(),
        })
    }

    /// Traefik's middleware counting by the peer.
    pub fn direct(&self) -> Value {
        self.middleware(json!({"ipv6Subnet": 64}))
    }

    /// Traefik's middleware counting by the visitor a trusted proxy names
    /// (the last `X-Forwarded-For` entry).
    pub fn proxied(&self) -> Value {
        self.middleware(json!({"depth": 1, "ipv6Subnet": 64}))
    }

    fn middleware(&self, strategy: Value) -> Value {
        json!({"rateLimit": {
            "average": self.average,
            "period": self.period,
            "burst": self.burst,
            "sourceCriterion": {"ipStrategy": strategy},
        }})
    }
}

/// Cloudflare's ranges, parsed.
pub fn cloudflare() -> Vec<Cidr> {
    CLOUDFLARE
        .iter()
        .map(|c| Cidr::parse(c).expect("Cloudflare's ranges parse"))
        .collect()
}

/// Whether two networks share an address.
pub fn overlaps(a: &Cidr, b: &Cidr) -> bool {
    let prefix = a.prefix.min(b.prefix);
    match (a.address, b.address) {
        (std::net::IpAddr::V4(x), std::net::IpAddr::V4(y)) => {
            let mask = if prefix == 0 {
                0
            } else {
                u32::MAX << (32 - prefix)
            };
            u32::from(x) & mask == u32::from(y) & mask
        }
        (std::net::IpAddr::V6(x), std::net::IpAddr::V6(y)) => {
            let mask = if prefix == 0 {
                0
            } else {
                u128::MAX << (128 - prefix)
            };
            u128::from(x) & mask == u128::from(y) & mask
        }
        _ => false,
    }
}

/// Exempt addresses, one a line (or separated by commas or spaces), as
/// canonical networks; the first line refused says why. Not the whole
/// Internet, never one of Cloudflare's (that would exempt its visitors).
pub fn exempt(text: Option<&str>) -> Result<Vec<String>, String> {
    let cloudflare = cloudflare();
    let mut found: Vec<Cidr> = vec![];
    for word in text
        .unwrap_or_default()
        .split(['\n', ',', ' '])
        .map(str::trim)
        .filter(|w| !w.is_empty())
    {
        let cidr = Cidr::parse(word)?;
        if cidr.prefix == 0 {
            return Err(format!("{word}: not every address"));
        }
        if cloudflare.iter().any(|c| overlaps(c, &cidr)) {
            return Err(format!("{word} is Cloudflare's"));
        }
        if !found.contains(&cidr) {
            found.push(cidr);
        }
    }
    if found.len() > MAX_EXEMPT {
        return Err(format!("at most {MAX_EXEMPT} addresses"));
    }
    Ok(found.iter().map(ToString::to_string).collect())
}

/// Traefik's arguments trusting Cloudflare's edges on the public
/// entrypoints (the installer's too).
pub fn traefik_args() -> Vec<String> {
    let ranges = CLOUDFLARE.join(",");
    ["web", "websecure"]
        .iter()
        .map(|e| format!("--entrypoints.{e}.forwardedHeaders.trustedIPs={ranges}"))
        .collect()
}

/// At boot: Traefik trusts Cloudflare's edges on its public entrypoints
/// (restarted only when that changes), in the background; tried again
/// every minute until it is so (untrusted, their visitors would share one
/// count).
pub fn reconcile(app: &rustcms::AppState) {
    let app = app.clone();
    tokio::spawn(async move {
        let (Some(docker), Some(traefik)) = (
            app.service::<crate::services::docker_service::Docker>(),
            app.service::<crate::services::traefik_service::Traefik>(),
        ) else {
            return;
        };
        if traefik.service.is_empty() {
            return;
        }
        loop {
            match trust(&docker, &traefik).await {
                Ok(true) => tracing::info!("Traefik trusts Cloudflare's edges"),
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(%err, "Traefik's trusted proxies not set: again in a minute");
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    continue;
                }
            }
            return;
        }
    });
}

/// Traefik's public entrypoints trusting Cloudflare's edges, and nothing
/// else of their forwarded headers: whether it changed anything.
async fn trust(
    docker: &crate::services::docker_service::Docker,
    traefik: &crate::services::traefik_service::Traefik,
) -> crate::services::docker_service::DockerResult<bool> {
    let want = traefik_args();
    traefik
        .update_service(docker.engine(), |spec| {
            let args = spec
                .task_template
                .get_or_insert_with(Default::default)
                .container_spec
                .get_or_insert_with(Default::default)
                .args
                .get_or_insert_with(Vec::new);
            let before = args.clone();
            args.retain(|a| {
                !(a.starts_with("--entrypoints.web.forwardedHeaders.")
                    || a.starts_with("--entrypoints.websecure.forwardedHeaders."))
                    || want.contains(a)
            });
            for arg in &want {
                if !args.contains(arg) {
                    args.push(arg.clone());
                }
            }
            *args != before
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exempt_addresses() {
        assert_eq!(
            exempt(Some("10.8.0.5\n 192.168.1.7/24, 2001:db8::1/64\n10.8.0.5")).unwrap(),
            ["10.8.0.5/32", "192.168.1.0/24", "2001:db8::/64"]
        );
        assert_eq!(exempt(None).unwrap(), Vec::<String>::new());
        for bad in [
            "0.0.0.0/0",
            "::/0",
            "nope",
            "10.0.0.0/33",
            "104.16.1.1",
            "2606:4700::/16",
            "100.0.0.0/4",
        ] {
            assert!(exempt(Some(bad)).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..=MAX_EXEMPT).map(|i| format!("10.0.{i}.1")).collect();
        assert!(exempt(Some(&many.join("\n"))).is_err());
    }

    #[test]
    fn overlapping() {
        let p = |s| Cidr::parse(s).unwrap();
        assert!(overlaps(&p("10.0.0.0/8"), &p("10.1.2.3")));
        assert!(overlaps(&p("10.1.2.3"), &p("10.0.0.0/8")));
        assert!(!overlaps(&p("10.0.0.0/8"), &p("11.0.0.0/8")));
        assert!(!overlaps(&p("10.0.0.0/8"), &p("::/0")));
        assert!(overlaps(&p("2a06:98c0::/29"), &p("2a06:98c7::1")));
    }

    #[test]
    fn middlewares() {
        let limit = RateLimit {
            average: 50,
            period: "1s".into(),
            burst: 200,
            exempt: vec![],
        };
        assert_eq!(
            limit.direct(),
            json!({"rateLimit": {"average": 50, "period": "1s", "burst": 200,
                "sourceCriterion": {"ipStrategy": {"ipv6Subnet": 64}}}})
        );
        assert_eq!(
            limit.proxied()["rateLimit"]["sourceCriterion"]["ipStrategy"],
            json!({"depth": 1, "ipv6Subnet": 64})
        );
    }

    /// The installer trusts the same ranges.
    #[test]
    fn installer_trusts_cloudflare() {
        let installer = include_str!("../../install/install.sh");
        let line = format!("CLOUDFLARE_IPS=\"{}\"", CLOUDFLARE.join(","));
        assert!(installer.contains(&line), "install.sh's ranges differ");
        for entrypoint in ["web", "websecure"] {
            let arg =
                format!("--entrypoints.{entrypoint}.forwardedHeaders.trustedIPs=$CLOUDFLARE_IPS");
            assert!(installer.contains(&arg), "install.sh lacks {arg}");
        }
    }
}
