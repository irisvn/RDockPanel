//! What the firewall installs and runs on the host: `fw.sh` (every change,
//! under one lock, written to a journal first), the boot unit that loads
//! the confirmed table before the network and Docker, and Docker's drop-in
//! (Docker does not start unfiltered). Fixed texts: only the ruleset (on
//! standard input) and checked tokens vary (docs/firewall.md).

/// Where the host keeps it (root, 0700).
pub const DIR: &str = "/var/lib/rdockpanel/firewall";
/// The script, as the panel and systemd run it.
pub const FW: &str = "/bin/sh /var/lib/rdockpanel/firewall/fw.sh";

/// The host's side of every transaction. `apply TX REV` (the candidate on
/// standard input), `keep TX`, `revert TX`, `state TX`, `load [--verify]`,
/// `ifaces` (names on standard input), `status`, `disable`.
pub const FW_SH: &str = r#"#!/bin/sh
# RDockPanel's firewall on this host: written by the panel (docs/firewall.md).
set -eu
D=/var/lib/rdockpanel/firewall
J="$D/journal"
umask 077
mkdir -p "$D"
exec 9>>"$D/lock"
flock 9
touch "$J"
BOOT=$(cat /proc/sys/kernel/random/boot_id)

log() {
  printf '%s %s %s %s\n' "$1" "$2" "$BOOT" "$(cut -d' ' -f1 /proc/uptime)" >> "$J"
  sync "$J" 2>/dev/null || sync
}
state() { awk -v t="$1" '$1 == t { s = $2 } END { print s }' "$J"; }
boot_of() { awk -v t="$1" '$1 == t { b = $3 } END { print b }' "$J"; }
last_tx() { awk '$1 != "-" { t = $1 } END { print t }' "$J"; }
final() { case "$1" in kept|reverted|aborted|failed|"") return 0 ;; esac; return 1; }
token() { case "$1" in *[!a-z0-9]*|"") echo "bad token" >&2; exit 2 ;; esac; }
stop_timer() { systemctl stop "rdp-fw-revert-$1.timer" >/dev/null 2>&1 || true; }
snapshot() { # the table as the policy decides it: without the sets the host fills
  nft -s list table inet rdockpanel 2>/dev/null \
    | awk '/^\tset (docker_ifaces|icmp_v4|icmp_v6) \{/ { skip = 1 } skip && /^\t\}/ { skip = 0; next } !skip'
}

point() { # point the confirmed table at $1 (a file of this directory), atomically
  ln -sfn "$1" "$D/confirmed.next"
  mv -T "$D/confirmed.next" "$D/confirmed"
  sync "$D" 2>/dev/null || sync
}

finish_commit() { # the user's Keep is durable from "committing" on
  fc_rev=$(cat "$D/tx-$1/rev")
  if [ ! -f "$D/rev-$fc_rev.nft" ]; then
    cp "$D/tx-$1/candidate.nft" "$D/rev-$fc_rev.next"
    sync "$D/rev-$fc_rev.next" 2>/dev/null || sync
    mv "$D/rev-$fc_rev.next" "$D/rev-$fc_rev.nft"
  fi
  point "rev-$fc_rev.nft"
  rm -f "$D/boot-failed"
  log "$1" committed
  stop_timer "$1"
  log "$1" kept
}

restore() { # the transaction's own baseline: table and pointer
  rb=$(cat "$D/tx-$1/baseline" 2>/dev/null || echo none)
  if [ "$rb" = none ]; then
    nft delete table inet rdockpanel 2>/dev/null || true
    rm -f "$D/confirmed"
  else
    nft -f "$D/$rb"
    point "$rb"
  fi
  load_ifaces
  log "$1" reverted
}

load_ifaces() {
  [ -s "$D/docker-ifaces" ] || return 0
  nft list table inet rdockpanel >/dev/null 2>&1 || return 0
  {
    echo "flush set inet rdockpanel docker_ifaces"
    while read -r name; do
      case "$name" in ""|lo|*[!A-Za-z0-9_.-]*) continue ;; esac
      echo "add element inet rdockpanel docker_ifaces { \"$name\" }"
    done < "$D/docker-ifaces"
  } | nft -f -
}

recover() { # what an interrupted transaction leaves, settled (its own names:
  # a shell function shares the caller's variables)
  rtx=$(last_tx)
  [ -n "$rtx" ] || return 0
  rs=$(state "$rtx")
  case "$rs" in
    committing) finish_commit "$rtx" ;;
    committed) stop_timer "$rtx"; log "$rtx" kept ;;
    intended) rm -rf "$D/tx-$rtx"; log "$rtx" aborted ;;
    armed|applied)
      # Another boot: its timer is gone; what loads is the baseline.
      if [ "$(boot_of "$rtx")" != "$BOOT" ]; then
        log "$rtx" reverted
      fi ;;
  esac
}

cmd=${1:-}
case "$cmd" in
  apply)
    tx=$2; rev=$3; token "$tx"; token "$rev"
    recover
    last=$(last_tx); if [ -n "$last" ] && ! final "$(state "$last")"; then echo "another change is waiting: $last" >&2; exit 3; fi
    mkdir -p "$D/tx-$tx"
    cat > "$D/tx-$tx/candidate.nft"
    echo "$rev" > "$D/tx-$tx/rev"
    # The baseline is what runs now: the confirmed table, or the fallback when
    # it could not load (a damaged one is never what a revert restores).
    if [ -L "$D/confirmed" ] && [ ! -e "$D/boot-failed" ] && nft -c -f "$D/confirmed" >/dev/null 2>&1; then
      readlink "$D/confirmed" > "$D/tx-$tx/baseline"
    elif [ -L "$D/confirmed" ]; then
      echo fallback.nft > "$D/tx-$tx/baseline"
    else
      echo none > "$D/tx-$tx/baseline"
    fi
    sync "$D/tx-$tx" 2>/dev/null || sync
    log "$tx" intended
    if ! systemd-run --quiet --unit "rdp-fw-revert-$tx" --on-active=120s --timer-property=AccuracySec=1s /bin/sh "$D/fw.sh" revert "$tx" \
       || [ "$(systemctl is-active "rdp-fw-revert-$tx.timer" 2>/dev/null)" != active ]; then
      stop_timer "$tx"; log "$tx" failed; echo "the revert timer could not be armed" >&2; exit 4
    fi
    log "$tx" armed
    echo "armed=$(date +%s)"
    if ! nft -c -f "$D/tx-$tx/candidate.nft" || ! nft -f "$D/tx-$tx/candidate.nft"; then
      stop_timer "$tx"; log "$tx" failed; echo "the ruleset was refused" >&2; exit 5
    fi
    snapshot > "$D/tx-$tx/loaded"
    log "$tx" applied
    echo '== table'
    nft -j list table inet rdockpanel ;;
  keep)
    tx=$2; token "$tx"
    s=$(state "$tx")
    case "$s" in
      kept) exit 0 ;;
      committing|committed) finish_commit "$tx"; exit 0 ;;
      applied) ;;
      *) echo "not waiting ($s)" >&2; exit 6 ;;
    esac
    # Nothing loaded since (a reload, another change): the table is still its.
    if [ "$(tail -n 1 "$J" | cut -d' ' -f1,2)" != "$tx applied" ] \
       || [ "$(snapshot)" != "$(cat "$D/tx-$tx/loaded" 2>/dev/null)" ]; then
      echo "the table changed since it was applied" >&2; exit 6
    fi
    log "$tx" committing
    finish_commit "$tx" ;;
  revert)
    tx=$2; token "$tx"
    s=$(state "$tx")
    case "$s" in
      committing) finish_commit "$tx" ;;
      committed) stop_timer "$tx"; log "$tx" kept ;;
      intended|armed|applied) restore "$tx"; stop_timer "$tx" ;;
      *) : ;;
    esac ;;
  state)
    tx=$2; token "$tx"
    echo "state=$(state "$tx")"
    echo "boot=$(boot_of "$tx")"
    echo "now=$BOOT"
    echo "timer=$(systemctl is-active "rdp-fw-revert-$tx.timer" 2>/dev/null || true)" ;;
  load)
    recover
    if [ "${2:-}" = --verify ] && nft list table inet rdockpanel >/dev/null 2>&1; then exit 0; fi
    # Off only when there is no confirmed link at all; a broken one is on.
    [ -L "$D/confirmed" ] || [ -e "$D/confirmed" ] || exit 0
    rm -f "$D/boot-failed"
    log - loaded
    if ! nft -f "$D/confirmed"; then
      echo confirmed > "$D/boot-failed"
      nft -f "$D/fallback.nft" || { echo both > "$D/boot-failed"; exit 1; }
    fi
    load_ifaces ;;
  ifaces)
    tmp="$D/docker-ifaces.next"
    : > "$tmp"
    while read -r name; do
      case "$name" in ""|lo|*[!A-Za-z0-9_.-]*) continue ;; esac
      printf '%s\n' "$name" >> "$tmp"
    done
    mv "$tmp" "$D/docker-ifaces"
    load_ifaces ;;
  status)
    echo installed=yes
    [ -e "$D/confirmed" ] && echo confirmed=yes || echo confirmed=no
    tx=$(last_tx); echo "last=$tx $(state "$tx")"
    echo "boot_failed=$(cat "$D/boot-failed" 2>/dev/null || echo none)"
    echo '== table'
    nft -j list table inet rdockpanel 2>/dev/null || true ;;
  disable)
    last=$(last_tx); if [ -n "$last" ] && ! final "$(state "$last")"; then echo "a change is waiting: $last" >&2; exit 3; fi
    if nft list table inet rdockpanel >/dev/null 2>&1; then
      nft delete table inet rdockpanel || { echo "the table could not be removed" >&2; exit 7; }
    fi
    rm -f /etc/systemd/system/docker.service.d/rdockpanel-firewall.conf
    systemctl daemon-reload
    systemctl disable rdockpanel-firewall.service >/dev/null 2>&1 || true
    rm -f /etc/systemd/system/rdockpanel-firewall.service
    systemctl daemon-reload
    rm -rf "$D"/tx-* "$D"/rev-* "$D/confirmed" "$D/fallback.nft" "$D/docker-ifaces" "$D/boot-failed"
    log - disabled ;;
  *) echo "usage: fw.sh apply|keep|revert|state|load|ifaces|status|disable" >&2; exit 2 ;;
esac
"#;

/// Loads the confirmed table first thing (before the network and Docker).
pub const UNIT: &str = "[Unit]
Description=RDockPanel firewall (the panel's nftables table)
DefaultDependencies=no
After=local-fs.target
Wants=network-pre.target
Before=network-pre.target docker.service shutdown.target
Conflicts=shutdown.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/sh /var/lib/rdockpanel/firewall/fw.sh load

[Install]
WantedBy=sysinit.target
";

/// Docker waits for the firewall, and checks the table is there each time
/// it starts (an upgrade or a restart).
pub const DROPIN: &str = "[Unit]
Requires=rdockpanel-firewall.service
After=rdockpanel-firewall.service

[Service]
ExecStartPre=/bin/sh /var/lib/rdockpanel/firewall/fw.sh load --verify
";

/// Installs fw.sh, the fallback (on standard input), the unit and the
/// drop-in. Run once before the first apply; again it only rewrites them.
pub fn install_script() -> String {
    format!(
        r#"set -eu
umask 077
D={DIR}
mkdir -p "$D"
chmod 700 "$D"
cat > "$D/fw.sh.next" <<'RDP_FW_SH'
{FW_SH}RDP_FW_SH
chmod 700 "$D/fw.sh.next"
mv "$D/fw.sh.next" "$D/fw.sh"
cat > "$D/fallback.next"
nft -c -f "$D/fallback.next"
mv "$D/fallback.next" "$D/fallback.nft"
cat > /etc/systemd/system/rdockpanel-firewall.service <<'RDP_FW_UNIT'
{UNIT}RDP_FW_UNIT
mkdir -p /etc/systemd/system/docker.service.d
cat > /etc/systemd/system/docker.service.d/rdockpanel-firewall.conf <<'RDP_FW_DROPIN'
{DROPIN}RDP_FW_DROPIN
systemctl daemon-reload
systemctl enable rdockpanel-firewall.service >/dev/null 2>&1
systemctl start rdockpanel-firewall.service
echo installed
"#
    )
}

/// `fw.sh <args>` for the host (each argument a checked token).
pub fn fw(args: &[&str]) -> String {
    let mut line = FW.to_owned();
    for a in args {
        debug_assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        line.push(' ');
        line.push_str(a);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_script_embeds_the_files_whole() {
        let s = install_script();
        assert!(s.contains("cat > \"$D/fw.sh.next\" <<'RDP_FW_SH'\n#!/bin/sh\n"));
        assert!(s.contains("esac\nRDP_FW_SH\n"));
        assert!(s.contains("ExecStart=/bin/sh /var/lib/rdockpanel/firewall/fw.sh load\n"));
        assert!(s.contains(
            "ExecStartPre=/bin/sh /var/lib/rdockpanel/firewall/fw.sh load --verify\nRDP_FW_DROPIN"
        ));
        // No heredoc delimiter appears inside what it delimits.
        for d in ["RDP_FW_SH", "RDP_FW_UNIT", "RDP_FW_DROPIN"] {
            assert!(!FW_SH.contains(d) && !UNIT.contains(d) && !DROPIN.contains(d));
        }
        assert_eq!(
            fw(&["keep", "abc123"]),
            "/bin/sh /var/lib/rdockpanel/firewall/fw.sh keep abc123"
        );
    }
}

/// The files a VM test runs (`RDP_FW_VM_DIR=… cargo test vm_files -- --ignored`):
/// the install script, the fallback and policies for the VM's addresses.
#[cfg(test)]
mod vm {
    use crate::services::firewall_rules::{
        Cidr, HostFacts, Opening, Policy, Protocol, Target, fallback, render,
    };

    #[test]
    #[ignore]
    fn vm_files() {
        let dir = std::path::PathBuf::from(std::env::var("RDP_FW_VM_DIR").expect("RDP_FW_VM_DIR"));
        let me = std::env::var("RDP_FW_ME").unwrap_or_else(|_| "192.168.139.3".to_owned());
        let host = HostFacts {
            ssh_ports: vec![22],
            docker_ifaces: vec!["docker0".into(), "docker_gwbridge".into()],
        };
        let open = |id, port, target, sources: &[&str]| Opening {
            id,
            ports: (port, port),
            protocol: Protocol::Tcp,
            target,
            anywhere: false,
            sources: sources.iter().map(|s| Cidr::parse(s).unwrap()).collect(),
        };
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("install.sh"), super::install_script()).unwrap();
        std::fs::write(dir.join("fallback.nft"), fallback(&host)).unwrap();
        // a: 8080 (the mesh) and 9000 (the host) opened for this Mac only.
        let a = Policy {
            openings: vec![
                open(1, 8080, Target::Published, &[&me]),
                open(2, 9000, Target::Host, &[&me]),
            ],
            blocked: vec![Cidr::parse("192.0.2.0/24").unwrap()],
        };
        std::fs::write(dir.join("a.nft"), render(&a, &host)).unwrap();
        // b: nothing opened (8080 and 9000 closed).
        std::fs::write(dir.join("b.nft"), render(&Policy::default(), &host)).unwrap();
        // c: this Mac blocked (SSH still reaches; the web does not).
        let c = Policy {
            openings: vec![],
            blocked: vec![Cidr::parse(&me).unwrap()],
        };
        std::fs::write(dir.join("c.nft"), render(&c, &host)).unwrap();
    }
}
