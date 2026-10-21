# Firewall

The **Firewall** page (Workspace menu, `system.manage`) shows what the server listens on and runs its firewall:

- **Default deny inbound.** Every new inbound connection is closed, except:
  - SSH: every port the server's sshd listens on, TCP;
  - the web: TCP 80 and 443 (Traefik: the panel and every domain);
  - the **openings**: a port or range, TCP or UDP, for its sources only. Its IPv4 and IPv6 sources are listed apart, and an empty family lets none of it in. "Anywhere" is an explicit choice, shown in red.
- **Openings target** either *this server* (a program outside Docker) or *a port a service publishes*. A published port is opened on both paths Docker uses: its DNAT, matched on the port the client asked for, and its proxy.
- **Blocked addresses** reach nothing but SSH. That includes every site on 80/443, since the server cannot tell the panel's domain from an app's. Visitors through the Cloudflare tunnel are not affected: the server does not see their address.
- **Existing connections** continue until they close; the rules apply to new ones.
- **Off by default.** It is turned on by the first **Apply**. **Turn off** removes it entirely.

It is refused, with the reason, when the server does not fit the supported case:

- nft or systemd is missing;
- the Swarm has more than one node;
- a macvlan or ipvlan Docker network exists;
- `nftables.service` is enabled (its stock configuration flushes every table at boot);
- a Docker bridge carries the default route;
- the panel has no domain and no tunnel (port 3000 would close).

The firewall does not replace your provider's own firewall or security groups. The page's "Listening ports" says what the server listens on, not what the internet reaches.

## How it works

The panel owns one nftables table, `inet rdockpanel`, replaced as a whole (`nft -f`):

- **`input`** (priority filter − 5) handles the host's own ports, and docker-proxy for published ones. It accepts, in order:
  1. loopback and Docker's bridges (containers reach the host's listeners);
  2. established and related connections (invalid ones are dropped);
  3. essential ICMP and ICMPv6 (errors, neighbour and router discovery, MLD; echo is limited per source) and DHCP replies;
  4. SSH.

  It then drops the blocked addresses, accepts 80/443 and the openings, and drops everything else.
- **`forward`** (priority filter − 5) handles connections from outside to ports Docker publishes, after its DNAT: `ct direction original ct status dnat ct state new`. They jump to `inbound`, which:
  - drops the blocked addresses;
  - accepts 80/443 and the published openings (`ct original proto-dst`);
  - drops the rest.

  Traffic from Docker's bridges (container egress, between containers, hairpin) and replies go on untouched.
- **Docker's bridges** are the interfaces of its bridge networks, as Docker names them:
  - `docker0`, `docker_gwbridge`, `br-<id12>`, or a network's own `com.docker.network.bridge.name`;
  - never an interface with the default route.

  The set is refreshed when the panel creates a network or runs Compose, and every two minutes.
- Only drops are final. An accept in this table only means "not dropped here": Docker's, ufw's and firewalld's rules still apply after it.

## Changes are transactions

Each **Apply** is a transaction the host journals before it changes anything. Everything runs under one lock: `flock` on `/var/lib/rdockpanel/firewall/lock`, which is never removed.

1. The panel records the change (`intent`), renders the table from the saved policy and the host's facts, and sends it to `fw.sh apply <tx> <rev>`. The host then:
   1. refuses when another change is not over;
   2. saves the candidate and its baseline (the confirmed table, or none);
   3. arms its revert, a transient systemd timer at 120 s (`rdp-fw-revert-<tx>`), and checks that it is active;
   4. checks the candidate with `nft -c`, then loads it.

   Any failure before the load leaves the table as it was.
2. The page shows **Trying the new rules**: open the panel and SSH from a new connection (a private window, a new SSH session), then press **Keep**.
   - Keep checks that the host still has the table it loaded (canonical hash).
   - The host appends `committing` first, then repoints `confirmed` to `rev-<id>.nft`, then appends `committed`, and only then stops the timer (`kept`).
   - From `committing` on, every path finishes the commit, never reverts it.
3. Without Keep, the timer runs `fw.sh revert <tx>`. It restores that transaction's own baseline (the table and the `confirmed` pointer) and journals `reverted`. **Go back now** does the same at once.

An interruption is settled from the host's journal:

| What happened | What the host and the panel do |
|---|---|
| The panel restarted | Its next look adopts the host's state; an expired change is reverted. |
| The host rebooted | The boot loader loaded the confirmed table, which is the baseline of a change never kept. |
| A request timed out | The change stays "unknown" until the journal says how it ended. |

## At boot

`rdockpanel-firewall.service` loads the confirmed table before the network and Docker (`DefaultDependencies=no`, `Before=network-pre.target docker.service`, wanted by `sysinit.target`). Docker's drop-in (`docker.service.d/rdockpanel-firewall.conf`) has two parts:

- `Requires=` and `After=` the unit;
- `ExecStartPre=fw.sh load --verify`: every Docker start (an upgrade, a restart) checks that the table is there.

If the confirmed table does not load, the **fallback** loads instead (SSH and the web only), and the page says so. If neither loads, Docker does not start, so no service is exposed unfiltered; SSH still works. Programs outside Docker are filtered only while a table is loaded.

To recover by hand when the firewall's files are gone:

```sh
rm /etc/systemd/system/docker.service.d/rdockpanel-firewall.conf && systemctl daemon-reload
```

**Drift:** every two minutes the panel compares the table with the confirmed one. A change made outside the panel, `nft flush ruleset` for example, is reloaded and recorded as a `firewall.reconcile` action.

## On the host

`/var/lib/rdockpanel/firewall/` (root, 0700) holds:

| Path | What |
|---|---|
| `fw.sh` | the host script |
| `journal` | one line per step: transaction, step, boot id, uptime |
| `rev-<id>.nft` | every confirmed table, immutable |
| `confirmed` | a symlink to the current one |
| `fallback.nft` | the table loaded when the confirmed one cannot be |
| `tx-<tx>/` | each transaction's candidate and baseline |
| `docker-ifaces` | the trusted bridges |
| `boot-failed` | present when a boot fell back |

**Turn off** (`fw.sh disable`) removes, in order:

1. the drop-in (and `daemon-reload`);
2. the unit;
3. the table;
4. the files (the journal stays).

Do it before uninstalling the panel ([install.md](install.md#uninstalling)).

The panel reads and changes the host only through fixed scripts. They run in the host's namespaces (`nsenter -t 1`) from a short-lived privileged helper container of its own image, which is removed after each command, as Reboot does. Only the rendered table (on standard input) and checked tokens vary: transaction ids are lowercase letters and digits, and bridge names are checked interface names. Notes are never rendered into the table.
