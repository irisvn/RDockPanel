# Install on macOS

Run RDockPanel on a Mac with Apple silicon, with the same installer and the same image as on a
Linux server.

## Before you start

- A Mac with **Apple silicon**.
- **[OrbStack](https://orbstack.dev)** or **Docker Desktop**, started, and set to start at login.
  It is tested with OrbStack; Docker Desktop should work the same way (its Swarm mode on).
- Ports **80** and **443** free on the Mac (and **3000** while the panel has no domain).

::: tip License
OrbStack and Docker Desktop need a paid license for some company use: check theirs.
:::

## Install

1. In a terminal, **as yourself, without `sudo`**:

   ```sh
   curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
     | sh -s -- --admin-email you@example.com
   ```

2. Note the administrator's password it prints.
3. Open http://localhost:3000/admin and sign in. From other devices on the same network, use
   `http://<the Mac's address>:3000` while the Mac keeps that address.

The options are the same as on Linux (`--version`, `--database`, `--domain`…): see
[Install on Linux](/guide/install#options). Run the same command again to upgrade or change them;
**Settings → Version → Update** works as on a server.

## Where things are

- `~/.rdockpanel` holds what `/etc/rdockpanel` holds on Linux: `install.conf`, `secrets/`,
  Traefik's routes and certificates, plus `backups/` (local backups and the update's database
  copies) and `compose/` (Compose services' files).
- The panel's database is in a Docker volume: `rdp-mysql-data` (MySQL) or `rdp-panel-data`
  (SQLite).

## What differs from a Linux server

- **Docker runs in a Linux VM.** The "server" the panel shows (CPU, memory, disk, network) is that
  VM, and the memory check before a deploy counts the VM's memory, never the whole Mac's.
- **Resource limits come from Docker's settings.** OrbStack uses every CPU and takes memory as
  containers need it, up to its limit (OrbStack's Settings → System, or
  `orb config set memory_mib 12288`, then restart OrbStack). Docker Desktop's are in its Settings →
  Resources (CPUs, memory, swap, disk). Leave macOS a few GB.
- **No Firewall and no Reboot.** They would act on the VM. Use the Mac's own firewall (System
  Settings → Network → Firewall), and restart Docker or the Mac yourself.
- **The Mac must be awake and Docker running** for your apps to answer. When Docker starts again,
  Swarm starts the panel and your services again.
- **No public address.** A Mac behind a home or office router cannot be reached by Let's Encrypt
  over HTTP. For real domains use [Cloudflare Tunnel](/apps/tunnel) (no port to open) or Let's
  Encrypt over DNS (Settings → Certificates). Or use names such as `app.localhost`, reached on the
  Mac itself.
- **Host folders**: a service's host directory sees the VM's paths; only folders under `/Users`
  are the Mac's. Host directories stay off unless an administrator allows a folder (see
  [Storage and volumes](/apps/storage)).
- **Refresh IP** finds the address of the network the Mac is on, not a server's.

## Good to know

- The installer stops when a port it needs is taken, naming the program. A program run by another
  user (root included) may not be seen: Docker then fails to start Traefik.
- Do not run an installed panel and a development copy (`rustcms serve`) on the same Docker: the
  installer stops when it finds the development Traefik.

## Uninstall

Follow [Uninstall on Linux](/guide/install#uninstall) **without `sudo`**, with these changes:

- once the panel's container is gone (a few seconds after `docker service rm`), remove the volume
  `rdp-panel-data` (SQLite) or `rdp-mysql-data` (MySQL);
- remove `~/.rdockpanel` instead of `/etc/rdockpanel` and `/var/lib/rdockpanel/…`, but keep
  `~/.rdockpanel/compose` until your Compose services and their data are removed.
