//! Where a domain's name leads: this server, Cloudflare (proxied or a
//! tunnel's public hostname), elsewhere, or nowhere yet. The domain form and
//! the Domains page say so: through a Cloudflare Tunnel, a domain gets its
//! HTTPS from Cloudflare, and Let's Encrypt's HTTP check for this server's
//! certificate stops at Cloudflare. Proxied to this server (Cloudflare's
//! Full mode) it still needs one: a DNS resolver or an uploaded certificate.
//! Only said, never enforced. Answers are kept [`KEPT`] (several lookups a
//! page).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

/// How long an answer is kept.
const KEPT: Duration = Duration::from_secs(300);
const LOOKUP_TIME: Duration = Duration::from_secs(3);

/// Cloudflare's published ranges (cloudflare.com/ips).
const CLOUDFLARE_V4: [(u32, u8); 15] = [
    (0xADF5_3000, 20), // 173.245.48.0/20
    (0x6715_F400, 22), // 103.21.244.0/22
    (0x6716_C800, 22), // 103.22.200.0/22
    (0x671F_0400, 22), // 103.31.4.0/22
    (0x8D65_4000, 18), // 141.101.64.0/18
    (0x6CA2_C000, 18), // 108.162.192.0/18
    (0xBE5D_F000, 20), // 190.93.240.0/20
    (0xBC72_6000, 20), // 188.114.96.0/20
    (0xC5EA_F000, 22), // 197.234.240.0/22
    (0xC629_8000, 17), // 198.41.128.0/17
    (0xA29E_0000, 15), // 162.158.0.0/15
    (0x6810_0000, 13), // 104.16.0.0/13
    (0x6818_0000, 14), // 104.24.0.0/14
    (0xAC40_0000, 13), // 172.64.0.0/13
    (0x8300_4800, 22), // 131.0.72.0/22
];
const CLOUDFLARE_V6: [(u128, u8); 7] = [
    (0x2400_cb00 << 96, 32),
    (0x2606_4700 << 96, 32),
    (0x2803_f800 << 96, 32),
    (0x2405_b500 << 96, 32),
    (0x2405_8100 << 96, 32),
    (0x2a06_98c0 << 96, 29),
    (0x2c0f_f248 << 96, 32),
];

/// Whether `ip` is one of Cloudflare's.
pub fn is_cloudflare(ip: IpAddr) -> bool {
    // `::ffff:104.21.26.135` is the IPv4 address.
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        ip => ip,
    };
    match ip {
        IpAddr::V4(v4) => {
            let n = u32::from(v4);
            CLOUDFLARE_V4
                .iter()
                .any(|(net, bits)| n >> (32 - bits) == net >> (32 - bits))
        }
        IpAddr::V6(v6) => {
            let n = u128::from(v6);
            CLOUDFLARE_V6
                .iter()
                .any(|(net, bits)| n >> (128 - bits) == net >> (128 - bits))
        }
    }
}

/// Where a name leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    /// This server's address (among its answers).
    Server,
    /// Only Cloudflare's addresses: proxied, or a tunnel's hostname.
    Cloudflare,
    Elsewhere,
    /// It does not resolve.
    None,
}

/// Where `addresses` lead, said against this server's `public_ip`.
pub fn via_of(addresses: &[IpAddr], public_ip: &str) -> Via {
    if addresses.is_empty() {
        Via::None
    } else if addresses.iter().any(|a| a.to_string() == public_ip) {
        Via::Server
    } else if addresses.iter().all(|a| is_cloudflare(*a)) {
        Via::Cloudflare
    } else {
        Via::Elsewhere
    }
}

static KEPT_ANSWERS: Mutex<Option<HashMap<String, (Instant, Via)>>> = Mutex::new(None);
/// Lookups at once (each a blocking resolver call, its slot held until it
/// ends, even past the timeout: never the whole blocking pool).
static LOOKUPS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(8)));
/// Names kept at most.
const KEPT_MAX: usize = 1000;

/// Where `host` leads now (a kept answer within [`KEPT`]).
pub async fn via(host: &str) -> Via {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if let Some(map) = KEPT_ANSWERS.lock().expect("not poisoned").as_ref()
        && let Some((at, via)) = map.get(&host)
        && at.elapsed() < KEPT
    {
        return *via;
    }
    // Within the timeout, the wait for a slot too (all held by slow names:
    // this one is not answered, never waited for).
    let name = host.clone();
    let lookup = async move {
        let slot = LOOKUPS.clone().acquire_owned().await.ok()?;
        tokio::task::spawn_blocking(move || {
            let _slot = slot;
            use std::net::ToSocketAddrs;
            (name.as_str(), 443)
                .to_socket_addrs()
                .map(|found| found.map(|a| a.ip()).collect::<Vec<_>>())
        })
        .await
        .ok()?
        .ok()
    };
    // Not answered (timed out, no slot, the resolver failing): said as
    // unresolved now, never kept (asked again next time).
    let Ok(Some(addresses)) = tokio::time::timeout(LOOKUP_TIME, lookup).await else {
        return Via::None;
    };
    let via = via_of(&addresses, &crate::services::system_service::public_ip());
    let mut kept = KEPT_ANSWERS.lock().expect("not poisoned");
    let map = kept.get_or_insert_with(HashMap::new);
    // Bounded: past it, the old answers go, then all if still too many.
    if map.len() >= KEPT_MAX {
        map.retain(|_, (at, _)| at.elapsed() < KEPT);
        if map.len() >= KEPT_MAX {
            map.clear();
        }
    }
    map.insert(host, (Instant::now(), via));
    via
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn where_a_name_leads() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(is_cloudflare(ip("104.21.26.135")));
        assert!(is_cloudflare(ip("172.67.136.75")));
        assert!(is_cloudflare(ip("2606:4700:3036::6815:1a87")));
        assert!(!is_cloudflare(ip("103.90.224.55")));
        assert!(is_cloudflare(ip("::ffff:104.21.26.135")));
        assert!(
            !is_cloudflare(ip("1.1.1.1")),
            "Cloudflare's resolver, not its proxy ranges"
        );
        assert_eq!(via_of(&[], "103.90.224.55"), Via::None);
        assert_eq!(via_of(&[ip("103.90.224.55")], "103.90.224.55"), Via::Server);
        assert_eq!(
            via_of(&[ip("104.21.26.135"), ip("172.67.136.75")], "103.90.224.55"),
            Via::Cloudflare
        );
        assert_eq!(
            via_of(&[ip("104.21.26.135"), ip("8.8.8.8")], "103.90.224.55"),
            Via::Elsewhere
        );
    }
}
