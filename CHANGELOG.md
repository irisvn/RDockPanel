# Changelog

Images: `ghcr.io/irisvn/rdockpanel:X.Y.Z` (also `X.Y` and `latest`); every push to `main` is
`main` and `main-<commit>`. Install or upgrade a release with `install.sh --version X.Y.Z`.

## 0.14.0 — 2026-11-18

- **Docker → Nodes**: the Swarm's servers with their state, resources and tasks. Add a server (a
  command to copy), pause, drain (what moves is said first), activate, or remove one that has left.
  The panel's own server is never paused, drained or removed.
- Environment placeholders are now `$(RDP_DOMAIN)`, `$(RDP_HOST)` and `$(RDP_HOST_IP)`; a template's
  environment and domains are saved with them. The names used before keep working.
- cloudflared 2026.11.1 (the newest) for Cloudflare Tunnel.
- A user guide in English and Vietnamese, with screenshots: https://irisvn.github.io/RDockPanel/

## 0.13.1 — 2026-11-17

- RustCMS 0.13.1: a record's page (Admin → its record) is drawn from its form, and an encrypted field no
  longer says whether it is set there.

## 0.13.0 — 2026-11-17

- An imported PostgreSQL, MySQL, MariaDB or MongoDB container can become a database service of the panel
  (connection details, dumps, backups), its image, settings and data kept as they are. Chosen in the
  container's details.
- An imported domain behind a Traefik IP allow list keeps it, as a middleware of the panel (shown in
  Settings → Traefik) instead of being left out.
- Choose where a service runs (Advanced tab, administrators only): automatic, one node of the Swarm, or any
  node. Backups, scripts, the terminal and live stats work for containers on the panel's node; the page
  says so when a service runs elsewhere.
- RustCMS 0.13.0, with SQLite 3.54 (a security fix for SQLite databases).

## 0.12.1 — 2026-11-17

- Importing a database no longer stops at its health check: a password written in it (`mysqladmin ping
  -p…`, `redis-cli -a …`) moves to an environment variable the check reads. A port (`pg_isready -p 5432`),
  a database name or a `$VARIABLE` is no longer taken for a password. A check the panel cannot rewrite can be
  dropped or replaced in the container's details.
- A health check keeps its timeout and start period; a service's health check can be turned off (the
  image's own does not run either). Both are in the service's Advanced tab.
- An imported container keeps its capabilities, sysctls, groups and CPU limit; the settings the panel does
  not carry (shared memory, ulimits, tmpfs, devices, DNS, working directory…) are said in the plan, and a
  container run without its image's entrypoint is not imported.
- The plan says how much data it copies, the free room and about how long the services are down, and an
  import that does not fit is refused before it starts.
- The import page shows what each step did (stopped, copied, deployed…).
- A partial import of a Compose project says that `docker compose up` would start the original again.
- A port the panel keeps (80, 443…) or one on the loopback can be published on another port, chosen in the
  plan.

## 0.12.0 — 2026-11-17

Contributed by xuandung38 (pull request #1), reviewed and completed.

- **Import existing containers** (docker run or docker compose) into a project as ordinary services:
  the panel reads them and the ones they need, shows a plan with what blocks and what is lost, stops
  the originals, copies their volumes, deploys services of its own and keeps the originals so the
  whole thing can be undone. Buttons: Import containers (Dashboard and projects), Bring into the panel
  (Docker → Containers). The guide is docs/import.md.
- A folder on the machine is used in place; **Move all** (a service's Storage tab) later copies it into a
  volume, and **Clean the old folder** deletes it. A folder that another container uses, or a place the
  panel never gives a service (the Docker socket, `/proc`, the panel's own folder), is never imported,
  moved or deleted. Once its old folder is being cleaned, a move can no longer be undone.
- The new services keep the domains of the containers' Traefik labels and `VIRTUAL_HOST`. A domain behind
  Traefik basic auth brings its users along; one behind another protection (forwardAuth, an allow list…)
  is left out, so a protected site is never served open. A read-only mount stays read only.
- An imported container keeps the user it ran as, and a password on its command line moves to an
  environment variable (when its image runs the command it is given). Such a service, and one running an
  image only this machine has, stays the administrators' and runs on this machine.
- **Several Cloudflare Tunnels**: one cloudflared per tunnel (one per token), added, switched on or off
  and deleted in Settings → Cloudflare Tunnel. The tunnel you had becomes `default`. A cloudflared container
  can become a tunnel of the panel when it is imported.
- A container's details (Docker → Containers → Inspect) no longer show passwords written in its command
  line, its health check, its labels or its log options. The container list hides secret labels too.
- A service with a writable volume or folder no longer rolls with "zero downtime" (two tasks would write
  the same data at once): it stops the old task first.
- A service cannot be named `rdockpanel` or `rdp-…`: those are the panel's own.
- Removing an image or pruning no longer takes an image or a volume that an import still holds.
- Before going back to an older release, keep the tunnel `default` on (or remove the other tunnels) and
  finish or undo the imports under way: see docs/development.md.

## 0.11.0 — 2026-11-12

- The panel installs on a **Mac with Apple silicon** (OrbStack or Docker Desktop): the same
  installer, run without `sudo`, and the same image as on a Linux server. Firewall and Reboot are
  for Linux servers and are not offered there. See docs/install.md.
- The installer stops, before changing anything, when it finds a Traefik it did not make (the
  development one), instead of keeping it.
- Giving the panel a domain with the installer now closes port 3000 (it stayed open on an install
  that had no domain before).
- The Cloudflare Tunnel no longer routes the panel when it has no domain of its own.
- A service can never bind-mount the installer's own directory.
- The versions line under Settings → Panel no longer runs off a phone's screen.
- The README is shorter: an overview, screenshots and quick starts; every feature is in
  docs/features.md.

## 0.10.1 — 2026-11-05

- The Domains page no longer lets long names and addresses run into the next column: they are cut
  and shown in full on hover, and the count under the table includes the panel's own domain.
- The homepage lists the newer features (firewall, one-click update, the memory check before a
  deploy, Compose, certificates and Cloudflare Tunnel, MySQL or SQLite), how to turn HTTPS on at
  install, and the license. Its screenshots come from 0.10.
- The template picker shows only the number of templates, not where they come from.
- Built on RustCMS 0.12.1 (from 0.11.0). Nothing changes in how the panel looks or works.

## 0.10.0 — 2026-10-29

- The panel can keep its own data in **SQLite** (one file, no database server: less memory on a
  small server) instead of MySQL. The installer asks at the first install, or takes
  `--database mysql|sqlite`; existing installs stay on MySQL. Settings → Version shows which one
  runs, and its Update button backs SQLite up too.
- Built on RustCMS 0.11.0 (from 0.10.3). A form's choice list now accepts only its exact values
  (a value with extra spaces is refused); framework cleanups appear in System → Schedules.
- Saving a service's environment, members, Git providers and a few other settings while another
  change is being written now waits its turn instead of failing.

## 0.9.3 — 2026-10-24

- Built on RustCMS 0.10.3 (from 0.8.2): framework fixes and security updates (`ctutils` 0.4.5,
  `tokio` 1.53.3). The panel looks and works as before.

## 0.9.2 — 2026-10-23

- A Compose file over 64 KB, and long notes in Vietnamese or emoji, are saved instead of failing
  (MySQL columns widened).
- An environment over 32 KB, or a service's files over 32 KB together, are refused with a clear
  message (the Environment tab, the Storage tab, templates, Deploy) instead of a server error.
- Templates and "Create from Schema" check what the forms check: domains are stored lowercase, and a
  command, a redirect or a repository path too long is refused by its field.
- Every firewall change, certificate upload, renewal and removal, and API token is recorded in the
  audit log together with the change; reading a service's environment through the API is refused
  when its audit entry cannot be written.
- A `[panel]` setting of the wrong kind (`RUSTCMS_PANEL__…`) now stops the panel at start with the
  setting's name, instead of running on defaults.

## 0.9.1 — 2026-10-22

- The memory warning is drawn like the rest of the panel, in light and dark.
- The memory estimate now recognizes an image written with or without `:latest` as the same one.
- Firewall: blocked addresses are a table with Unblock; Apply moves to the page header; the change
  being tried shows inside the Firewall section; buttons say when they are working.
- Maintenance: turning it on asks first; replacing your HTML with the default page asks first; a
  service's logo shows at its size again; Save sits at the bottom of the section.
- Rate limit: Save cannot be pressed twice; the help text moves under the limit.

## 0.9.0 — 2026-10-22

- **Room on the server**: creating a service, using a template, Deploy, Start, Restart and Force
  rebuild say when the server's memory (or disk) runs low or is not enough, with what each service
  would take. When it does not fit, deploying asks to be confirmed; a memory reservation the server
  cannot give is refused (Swarm would leave it pending). Deployments from the API, webhooks and
  trigger URLs are never stopped: their log says it, and the notification channels hear it.
- Templates keep their memory limit and reservation (EasyPanel's `resources`).

## 0.8.0 — 2026-10-22

- **Maintenance page** redesigned: simpler, light and dark, in the visitor's language, with when the
  service is back, a contact and a logo.
- Write your own maintenance page in HTML and CSS (scripts are blocked), with a live preview, or set
  one shared page for every service in Settings → Maintenance page.
- Addresses that still reach the service while it is in maintenance (you, your team, a VPN).

## 0.7.0 — 2026-10-22

- **Rate limit** in a service's Security tab: requests per visitor (their IP) on its domains, with
  presets or your own numbers, and addresses exempt from it (a VPN, a monitor). Visitors through
  Cloudflare (a proxied record or the tunnel) are counted by their own address. Applied when saved.
- Traefik now trusts `X-Forwarded-For` from Cloudflare's edges and from the tunnel: services see
  their visitors' address there. Upgrading restarts Traefik once.

## 0.6.1 — 2026-10-22

- The Firewall page no longer says ufw is on when it is installed but disabled.

## 0.6.0 — 2026-10-22

- **Firewall** page (Workspace menu): every port is closed to the internet except SSH, 80 and 443;
  open a port for the addresses you choose (a VPN), block addresses, see what listens and who owns
  it. Each change is tried first: keep it within two minutes or the server goes back by itself.
  Off until you turn it on.

## 0.5.5 — 2026-10-21

- The homepage's screens open in a viewer (previous / next, arrow keys, swipe) instead of a new page,
  and show the panel as it is now; the docs use the same new screenshots.
- Nixpacks is removed: apps build from Git with their Dockerfile. A service that used Nixpacks now
  builds with its repository's Dockerfile.
- The docs follow the code again (every admin route, the installer, permissions, Settings' tabs).

## 0.5.4 — 2026-10-21

- A homepage at the panel's address: what RDockPanel is, real screens, how to install it and a
  link to GitHub, in English or Vietnamese. It can be turned off in Settings → Panel.
- RustCMS Core 0.8.2 (the Stop/Start error page fixed at its source) and TLS security updates
  (rustls-webpki, webpki-roots).

## 0.5.3 — 2026-10-21

- Stop, Start or another action from a service's tab no longer shows "Something went wrong" for
  a moment (a reload was not needed).
- On a server under 2 GB of memory, the installer starts MySQL with less memory (no
  `performance_schema`, no binary log, smaller buffers) and gives it time to shut down. A rerun
  applies it: MySQL restarts once.

## 0.5.2 — 2026-10-21

- A domain whose name leads to Cloudflare is marked "via Cloudflare". With the tunnel on, its
  form suggests turning HTTPS off: through the tunnel, Cloudflare already gives it HTTPS.
- The installer no longer leaves Traefik a moment without the panel's route when it runs again.

## 0.5.1 — 2026-10-20

- Release notes in Settings → Version are shown formatted (bold, code, lists, links).

## 0.5.0 — 2026-10-20

- **Settings → Version**: see what the panel runs, whether a newer release is out, what changed,
  and update with one button (the database is backed up first; a failed update goes back by
  itself). The Dashboard says when a new release is out.
- **Old versions** of the panel can be removed to free disk space.
- **Domains, redirects, basic auth and maintenance apply at once** when saved, without a Deploy.
- **Every dialog has a close button.** The Queue keeps a history; a new Schedules page.
- **Cloudflare Tunnel** fixed and tested for real; cloudflared updated.
- **Traefik 3.7.**
- Running a console command no longer closes open terminals.

## 0.4.2 — 2026-10-20

On RustCMS Core 0.7.0.

- **New look** ([docs/ui-guidelines.md](docs/ui-guidelines.md)): every page on one set of rules
  (button sizes, one primary action per area, a ⋯ menu per row, a type scale, statuses with the
  same words and colors in light and dark), the panel's own dialogs instead of the browser's, and
  tables that page and scroll on a phone.
- The Docker pages (Containers, Images, Volumes, Networks) and the live part of a service are React
  pages like the rest: logs with a line count and Follow, a terminal with New session.
- Images are removed by any of their names (`DELETE /admin/api/x/docker/images?image=…`).
- No breadcrumb: a project or a service names its kind above its title, without links.
- New screenshots in the README.

## 0.4.1 — 2026-10-20

On RustCMS Core 0.7.0.

- **Branding** (Settings): the panel's name, primary color, logos and favicon while it runs, on
  the sign-in page too (images PNG, WebP or ICO up to 512 KB; no SVG).
- **Roles page:** the panel's permissions in areas with labels and descriptions, and four presets.
- The system line (versions, image, host) moved to the foot of Settings → Panel.
- Images' version label without the tag's `v` (`0.4.1`, as the panel shows it).

## 0.4.0 — 2026-10-20

The first tagged release, on RustCMS Core 0.6.1.

- **Server and Docker:** live CPU, memory, disk and network; containers, images, volumes and
  networks with logs and a terminal; prunes that never touch the panel's own objects.
- **Apps like EasyPanel:** projects of services (app, PostgreSQL, MySQL, MariaDB, MongoDB, Redis,
  Compose), deployed to Docker Swarm with zero downtime and rollback; builds from Git and GitHub
  (Dockerfile, Nixpacks); EasyPanel's templates; domains and HTTPS through Traefik (Let's Encrypt
  over HTTP or DNS, uploaded certificates, custom middlewares); backups to a directory or S3;
  notifications (Discord, Slack, Telegram, email, webhooks).
- **Phase 4:** a Domains page; Panel, Server, Traefik and Docker menus (the panel's logs, restart,
  update check, Refresh IP, reboot); Force rebuild and recent errors (failures Swarm recovered from
  are marked "Recovered automatically"); GitHub tokens per project; more notification events;
  project members (viewer, developer, admin); a public API (`/api/v1`) with scoped tokens and the
  `rdp` command line; Cloudflare Tunnel in Settings.
- **Installer:** version tags, MySQL and Traefik restarted only when their configuration changes,
  the panel's healthcheck kept, a short line per try while MySQL starts.

