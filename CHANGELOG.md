# Changelog

Images: `ghcr.io/irisvn/rdockpanel:X.Y.Z` (also `X.Y` and `latest`); every push to `main` is
`main` and `main-<commit>`. Install or upgrade a release with `install.sh --version X.Y.Z`.

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

