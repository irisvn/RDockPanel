# Changelog

Images: `ghcr.io/irisvn/rdockpanel:X.Y.Z` (also `X.Y` and `latest`); every push to `main` is
`main` and `main-<commit>`. Install or upgrade a release with `install.sh --version X.Y.Z`.

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

