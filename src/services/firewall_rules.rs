//! The firewall's policy as the panel keeps it, checked, and rendered as the
//! panel's own nftables table (`inet rdockpanel`): default deny inbound,
//! SSH, 80 and 443 always open, the openings for their sources only, the
//! blocked addresses. Pure: no host, no database (docs/firewall.md).

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use serde::{Deserialize, Serialize};

/// Always open on TCP, whatever the policy: the web (Traefik: the panel and
/// every domain). SSH's ports are found on the host.
pub const WEB_PORTS: [u16; 2] = [80, 443];
/// The most sources an opening lists (each family).
pub const SOURCES_MAX: usize = 256;
/// The most openings and blocked addresses.
pub const OPENINGS_MAX: usize = 200;
pub const BLOCKED_MAX: usize = 1000;

/// A network, canonical (`10.8.0.0/24`, `2001:db8::/32`; an address alone is
/// its /32 or /128).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Cidr {
    pub address: IpAddr,
    pub prefix: u8,
}

impl Cidr {
    /// `text` as a network: its host bits cleared.
    pub fn parse(text: &str) -> Result<Cidr, String> {
        let text = text.trim();
        let (address, prefix) = match text.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (text, None),
        };
        let address: IpAddr = address
            .parse()
            .map_err(|_| format!("{text} is not an IP address or network"))?;
        let max = if address.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            None => max,
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= max)
                .ok_or_else(|| format!("{text}: the prefix is 0 to {max}"))?,
        };
        let address = match address {
            IpAddr::V4(v4) => {
                let mask = if prefix == 0 {
                    0
                } else {
                    u32::MAX << (32 - prefix)
                };
                IpAddr::V4(Ipv4Addr::from(u32::from(v4) & mask))
            }
            IpAddr::V6(v6) => {
                let mask = if prefix == 0 {
                    0
                } else {
                    u128::MAX << (128 - prefix)
                };
                IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask))
            }
        };
        Ok(Cidr { address, prefix })
    }

    pub fn is_v4(&self) -> bool {
        self.address.is_ipv4()
    }
}

impl std::fmt::Display for Cidr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix)
    }
}

/// Where an opening applies: this server's own listeners, or a port a
/// service publishes (through Docker's DNAT and its proxy alike).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Host,
    Published,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        }
    }
}

/// A port (or range) opened for its sources; everything not opened is
/// closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opening {
    pub id: i64,
    pub ports: (u16, u16),
    pub protocol: Protocol,
    pub target: Target,
    /// From anywhere (an explicit choice), else only `sources`.
    pub anywhere: bool,
    /// Both families, each its own list (an empty family: none of it).
    pub sources: Vec<Cidr>,
}

/// What the firewall renders: its openings and blocked addresses (the
/// enabled ones), with what the host told about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Policy {
    pub openings: Vec<Opening>,
    pub blocked: Vec<Cidr>,
}

/// What the host adds to a rendering: its SSH ports (always open) and its
/// Docker bridges (trusted: containers reach the host's listeners).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostFacts {
    pub ssh_ports: Vec<u16>,
    pub docker_ifaces: Vec<String>,
}

/// A port list (`5432`, `8000-8100`) as its range.
pub fn parse_ports(text: &str) -> Result<(u16, u16), String> {
    let text = text.trim();
    let one = |s: &str| {
        s.trim()
            .parse::<u16>()
            .ok()
            .filter(|p| *p >= 1)
            .ok_or_else(|| format!("{text}: a port is 1 to 65535"))
    };
    let (start, end) = match text.split_once('-') {
        Some((a, b)) => (one(a)?, one(b)?),
        None => {
            let p = one(text)?;
            (p, p)
        }
    };
    if start > end {
        return Err(format!("{text}: the range starts after it ends"));
    }
    Ok((start, end))
}

/// A Docker bridge's name as the kernel allows it (and nothing that could
/// pass for more than a name in the ruleset).
pub fn valid_ifname(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 15
        && name != "lo"
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

/// An opening that SSH or the web already has open (TCP): refused, it would
/// only seem to restrict them.
pub fn overlaps_always_open(ports: (u16, u16), protocol: Protocol, ssh: &[u16]) -> Option<u16> {
    if protocol != Protocol::Tcp {
        return None;
    }
    WEB_PORTS
        .iter()
        .chain(ssh)
        .copied()
        .find(|p| (ports.0..=ports.1).contains(p))
}

fn range(ports: (u16, u16)) -> String {
    if ports.0 == ports.1 {
        ports.0.to_string()
    } else {
        format!("{}-{}", ports.0, ports.1)
    }
}

fn elements<T: std::fmt::Display>(items: impl IntoIterator<Item = T>) -> Option<String> {
    let list: Vec<String> = items.into_iter().map(|i| i.to_string()).collect();
    (!list.is_empty()).then(|| format!(" elements = {{ {} }};", list.join(", ")))
}

/// The table, replaced as a whole when loaded (`nft -f`).
pub fn render(policy: &Policy, host: &HostFacts) -> String {
    let mut ssh: Vec<u16> = host.ssh_ports.clone();
    ssh.sort_unstable();
    ssh.dedup();
    if ssh.is_empty() {
        ssh.push(22);
    }
    let ifaces: Vec<String> = host
        .docker_ifaces
        .iter()
        .filter(|n| valid_ifname(n))
        .map(|n| format!("\"{n}\""))
        .collect();
    let v4: Vec<&Cidr> = policy.blocked.iter().filter(|c| c.is_v4()).collect();
    let v6: Vec<&Cidr> = policy.blocked.iter().filter(|c| !c.is_v4()).collect();
    let mut t = String::new();
    t.push_str("table inet rdockpanel\ndelete table inet rdockpanel\ntable inet rdockpanel {\n");
    let _ = writeln!(
        t,
        "  set docker_ifaces {{ type ifname;{} }}",
        elements(&ifaces).unwrap_or_default()
    );
    let _ = writeln!(
        t,
        "  set ssh_ports {{ type inet_service;{} }}",
        elements(&ssh).unwrap_or_default()
    );
    let _ = writeln!(
        t,
        "  set blocked_v4 {{ type ipv4_addr; flags interval;{} }}",
        elements(&v4).unwrap_or_default()
    );
    let _ = writeln!(
        t,
        "  set blocked_v6 {{ type ipv6_addr; flags interval;{} }}",
        elements(&v6).unwrap_or_default()
    );
    t.push_str(
        "  set icmp_v4 { type ipv4_addr; size 4096; flags dynamic,timeout; timeout 60s; }\n",
    );
    t.push_str(
        "  set icmp_v6 { type ipv6_addr; size 4096; flags dynamic,timeout; timeout 60s; }\n",
    );
    for o in &policy.openings {
        if o.anywhere {
            continue;
        }
        let ov4: Vec<&Cidr> = o.sources.iter().filter(|c| c.is_v4()).collect();
        let ov6: Vec<&Cidr> = o.sources.iter().filter(|c| !c.is_v4()).collect();
        let _ = writeln!(
            t,
            "  set o{}_v4 {{ type ipv4_addr; flags interval;{} }}",
            o.id,
            elements(&ov4).unwrap_or_default()
        );
        let _ = writeln!(
            t,
            "  set o{}_v6 {{ type ipv6_addr; flags interval;{} }}",
            o.id,
            elements(&ov6).unwrap_or_default()
        );
    }
    // The host's own ports (and docker-proxy's, for published ones).
    t.push_str("  chain input {\n    type filter hook input priority filter - 5; policy accept;\n");
    t.push_str("    iifname \"lo\" accept\n    iifname @docker_ifaces accept\n");
    t.push_str("    ct state established,related accept\n    ct state invalid drop\n");
    t.push_str(
        "    icmp type { destination-unreachable, time-exceeded, parameter-problem } accept\n",
    );
    t.push_str("    icmp type echo-request update @icmp_v4 { ip saddr limit rate 5/second burst 10 packets } accept\n");
    t.push_str("    icmp type echo-request drop\n");
    t.push_str("    icmpv6 type { destination-unreachable, packet-too-big, time-exceeded, parameter-problem } accept\n");
    t.push_str("    icmpv6 type { nd-router-solicit, nd-router-advert, nd-neighbor-solicit, nd-neighbor-advert, nd-redirect } ip6 hoplimit 255 accept\n");
    t.push_str("    icmpv6 type { mld-listener-query, mld-listener-report, mld-listener-done, mld2-listener-report } ip6 saddr fe80::/10 accept\n");
    t.push_str("    icmpv6 type echo-request update @icmp_v6 { ip6 saddr limit rate 5/second burst 10 packets } accept\n");
    t.push_str("    icmpv6 type echo-request drop\n");
    t.push_str("    udp sport 67 udp dport 68 accept\n");
    t.push_str("    ip6 saddr fe80::/10 udp sport 547 udp dport 546 accept\n");
    t.push_str("    tcp dport @ssh_ports accept\n");
    t.push_str("    ip saddr @blocked_v4 drop\n    ip6 saddr @blocked_v6 drop\n");
    t.push_str("    tcp dport { 80, 443 } accept\n");
    for o in &policy.openings {
        opening_rules(&mut t, o, "th dport");
    }
    t.push_str("    drop\n  }\n");
    // Connections from outside to ports Docker publishes (after its DNAT,
    // matched on the port asked for); containers and replies go on.
    t.push_str(
        "  chain forward {\n    type filter hook forward priority filter - 5; policy accept;\n",
    );
    t.push_str("    iifname @docker_ifaces accept\n    ct state established,related accept\n");
    t.push_str("    ct direction original ct status dnat ct state new jump inbound\n  }\n");
    t.push_str("  chain inbound {\n");
    t.push_str("    ip saddr @blocked_v4 drop\n    ip6 saddr @blocked_v6 drop\n");
    t.push_str("    meta l4proto tcp ct original proto-dst { 80, 443 } accept\n");
    for o in policy
        .openings
        .iter()
        .filter(|o| o.target == Target::Published)
    {
        opening_rules(&mut t, o, "ct original proto-dst");
    }
    t.push_str("    drop\n  }\n}\n");
    t
}

fn opening_rules(t: &mut String, o: &Opening, port: &str) {
    let l4 = o.protocol.as_str();
    let ports = range(o.ports);
    if o.anywhere {
        let _ = writeln!(t, "    meta l4proto {l4} {port} {ports} accept");
    } else {
        let id = o.id;
        let _ = writeln!(
            t,
            "    meta l4proto {l4} {port} {ports} ip saddr @o{id}_v4 accept"
        );
        let _ = writeln!(
            t,
            "    meta l4proto {l4} {port} {ports} ip6 saddr @o{id}_v6 accept"
        );
    }
}

/// What is loaded when the confirmed table cannot be: SSH and the web only.
pub fn fallback(host: &HostFacts) -> String {
    render(&Policy::default(), host)
}

/// `nft -j list table inet rdockpanel` reduced to what the policy decides:
/// no handles or metadata, the elements of the sets the host fills by
/// itself (meters, Docker bridges) left out, the others sorted; the rules'
/// order kept. Its SHA-256, hex.
pub fn canonical_hash(listing: &str) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut doc: serde_json::Value =
        serde_json::from_str(listing).map_err(|e| format!("not nft's JSON: {e}"))?;
    let items = doc
        .get_mut("nftables")
        .and_then(|v| v.as_array_mut())
        .ok_or("not nft's JSON: no nftables list")?;
    items.retain(|item| item.get("metainfo").is_none());
    for item in items.iter_mut() {
        strip(item);
        if let Some(set) = item.get_mut("set").and_then(|s| s.as_object_mut()) {
            let name = set
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("")
                .to_owned();
            if matches!(name.as_str(), "docker_ifaces" | "icmp_v4" | "icmp_v6") {
                set.remove("elem");
            } else if let Some(elem) = set.get_mut("elem").and_then(|e| e.as_array_mut()) {
                elem.sort_by_key(|e| e.to_string());
            }
        }
    }
    let text = serde_json::to_string(items).map_err(|e| e.to_string())?;
    let digest = Sha256::digest(text.as_bytes());
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

fn strip(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("handle");
            map.remove("comment");
            for v in map.values_mut() {
                strip(v);
            }
        }
        serde_json::Value::Array(list) => list.iter_mut().for_each(strip),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cidr(s: &str) -> Cidr {
        Cidr::parse(s).unwrap()
    }

    #[test]
    fn networks_are_canonical() {
        assert_eq!(cidr("10.8.0.7/24").to_string(), "10.8.0.0/24");
        assert_eq!(cidr("203.0.113.25").to_string(), "203.0.113.25/32");
        assert_eq!(cidr("2001:db8::1/32").to_string(), "2001:db8::/32");
        assert_eq!(cidr("::1").to_string(), "::1/128");
        assert!(Cidr::parse("10.0.0.0/33").is_err());
        assert!(Cidr::parse("example.com").is_err());
        assert!(Cidr::parse("10.0.0.0/8; drop").is_err());
    }

    #[test]
    fn ports_and_names() {
        assert_eq!(parse_ports("5432"), Ok((5432, 5432)));
        assert_eq!(parse_ports("8000-8100"), Ok((8000, 8100)));
        assert!(parse_ports("0").is_err());
        assert!(parse_ports("9-1").is_err());
        assert!(parse_ports("70000").is_err());
        assert!(
            valid_ifname("br-3f2a1b4c5d6e") && valid_ifname("docker0") && valid_ifname("appbr")
        );
        assert!(!valid_ifname("lo") && !valid_ifname("a\"b") && !valid_ifname("a-very-long-name0"));
        assert_eq!(
            overlaps_always_open((20, 25), Protocol::Tcp, &[22]),
            Some(22)
        );
        assert_eq!(overlaps_always_open((443, 443), Protocol::Udp, &[22]), None);
        assert_eq!(
            overlaps_always_open((5432, 5432), Protocol::Tcp, &[2222]),
            None
        );
    }

    #[test]
    fn the_rendered_table() {
        let policy = Policy {
            openings: vec![
                Opening {
                    id: 1,
                    ports: (5432, 5432),
                    protocol: Protocol::Tcp,
                    target: Target::Published,
                    anywhere: false,
                    sources: vec![cidr("10.8.0.0/24")],
                },
                Opening {
                    id: 2,
                    ports: (51820, 51820),
                    protocol: Protocol::Udp,
                    target: Target::Host,
                    anywhere: true,
                    sources: vec![],
                },
            ],
            blocked: vec![cidr("198.51.100.7"), cidr("2001:db8::/32")],
        };
        let host = HostFacts {
            ssh_ports: vec![22],
            docker_ifaces: vec![
                "docker0".into(),
                "docker_gwbridge".into(),
                "bad\"name".into(),
            ],
        };
        let t = render(&policy, &host);
        assert!(t.starts_with("table inet rdockpanel\ndelete table inet rdockpanel\n"));
        assert!(t.contains(
            "set docker_ifaces { type ifname; elements = { \"docker0\", \"docker_gwbridge\" }; }"
        ));
        assert!(!t.contains("bad"));
        assert!(
            t.contains("set o1_v4 { type ipv4_addr; flags interval; elements = { 10.8.0.0/24 }; }")
        );
        assert!(
            t.contains("set o1_v6 { type ipv6_addr; flags interval; }"),
            "an empty family: none of it"
        );
        assert!(t.contains("meta l4proto tcp th dport 5432 ip saddr @o1_v4 accept"));
        assert!(t.contains("meta l4proto tcp ct original proto-dst 5432 ip6 saddr @o1_v6 accept"));
        assert!(t.contains("meta l4proto udp th dport 51820 accept"));
        assert!(
            !t.contains("ct original proto-dst 51820"),
            "a host opening is not a published one"
        );
        // SSH before the blocked addresses, the web after them; drop last.
        let ssh = t.find("tcp dport @ssh_ports accept").unwrap();
        let blocked = t.find("ip saddr @blocked_v4 drop").unwrap();
        let web = t.find("tcp dport { 80, 443 } accept").unwrap();
        assert!(ssh < blocked && blocked < web);
        assert!(
            t.contains("elements = { 198.51.100.7/32 }")
                && t.contains("elements = { 2001:db8::/32 }")
        );
        let input_end = t.find("  chain forward").unwrap();
        assert!(
            t[..input_end].trim_end().ends_with("drop\n  }")
                || t[..input_end].contains("    drop\n  }")
        );
        if let Some(path) = std::env::var_os("RDP_FW_DUMP") {
            std::fs::write(path, &t).unwrap();
        }
        // Without anything: SSH (22 when none was found) and the web only.
        let f = fallback(&HostFacts::default());
        assert!(f.contains("set ssh_ports { type inet_service; elements = { 22 }; }"));
        assert!(!f.contains("set o"));
    }

    #[test]
    fn the_canonical_form_ignores_what_the_host_fills() {
        let a = r#"{"nftables":[{"metainfo":{"version":"1.0.2"}},{"table":{"family":"inet","name":"rdockpanel","handle":7}},
            {"set":{"family":"inet","name":"icmp_v4","table":"rdockpanel","handle":3,"elem":[{"elem":{"val":"1.2.3.4"}}]}},
            {"set":{"family":"inet","name":"blocked_v4","table":"rdockpanel","handle":4,"elem":["10.0.0.2","10.0.0.1"]}},
            {"rule":{"family":"inet","table":"rdockpanel","chain":"input","handle":9,"expr":[{"drop":null}]}}]}"#;
        let b = r#"{"nftables":[{"metainfo":{"version":"1.0.9"}},{"table":{"family":"inet","name":"rdockpanel","handle":2}},
            {"set":{"family":"inet","name":"icmp_v4","table":"rdockpanel","handle":5}},
            {"set":{"family":"inet","name":"blocked_v4","table":"rdockpanel","handle":6,"elem":["10.0.0.1","10.0.0.2"]}},
            {"rule":{"family":"inet","table":"rdockpanel","chain":"input","handle":11,"expr":[{"drop":null}]}}]}"#;
        assert_eq!(canonical_hash(a).unwrap(), canonical_hash(b).unwrap());
        let c = b.replace("10.0.0.2", "10.0.0.3");
        assert_ne!(canonical_hash(a).unwrap(), canonical_hash(&c).unwrap());
    }
}
