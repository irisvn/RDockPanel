# Changelog

Images: `ghcr.io/irisvn/rdockpanel:X.Y.Z` (also `X.Y` and `latest`); every push to `main` is
`main` and `main-<commit>`. Install or upgrade a release with `install.sh --version X.Y.Z`.

## 0.5.0 — 2026-10-20

On RustCMS Core 0.8.1 (the image runs `migrate --allow-newer` and `permission:sync` at its start).

- **Settings → Version**: what the panel runs (version, commit, image) and with what (Core,
  Traefik, MySQL, cloudflared, Docker, the OS); whether a newer release is out (the registry's
  `X.Y.Z` tags, checked every 6 hours or by hand), its notes and the installer command for it; the
  history of version changes, updates and restarts. The Dashboard says a newer release to the
  panel's administrators (each can hide it until the next).
- **Update button** (Settings → Version, `system.manage`): the panel never updates by itself; an
  administrator's Update moves it to the newest release. New deployments, backups and restores
  wait while those running end; the panel's database is dumped to
  `/var/lib/rdockpanel/backups/panel/`; Swarm replaces the panel (pinned to the registry's
  digest) and rolls back if the new version does not become healthy — the version rolled back to
  starts on the database as the new one left it (`migrate --allow-newer`); the next panel records
  how it ended. A plain installer run keeps a newer release the panel updated to.
- **Old versions removed** (Settings → Version): the stopped containers of the panel's services'
  old tasks and their images, the running and previous versions kept. The Images page says which
  images run, which only stopped containers keep, and which none uses.
- Releases now publish their GitHub Release (this file's section) once their image is named.
- **Every dialog has a close button (×)** (Core 0.7.1).
- **System → Queue** keeps a history of finished jobs, each described ("deployment #12", "pull
  nginx:1.27"); **System → Schedules** shows the panel's nine schedules, their last runs with a
  summary ("2 backups queued") and a Run now (`schedules.manage`). A deployment cut by a stop is
  never run again behind the user's back (it is marked interrupted).
- Deleting a domain or a redirect writes the routes again at once (Core's `after_delete`).
- A domain reached only through Cloudflare Tunnel can have HTTPS off (Cloudflare holds TLS, no
  certificate is asked for): its visitors over HTTPS are still said so to the service.
- **Applied when saved:** a service's domains, redirects, basic authentication and maintenance are
  written to Traefik at once, without a deployment (the running containers are untouched); they no
  longer count as changes waiting for Deploy. What is in the Swarm service (image, environment,
  ports, mounts, resources, command…) still needs a Deploy. A first domain added to a running
  service makes Traefik join its project's network (Traefik restarts, as a deployment did). A
  Compose service's domains still apply by deploying (they are checked against its file then).
  Upgrading applies the route changes that were waiting for a Deploy.
- **Traefik v3.7** (from v3.5): the installer moves it at the next upgrade (one restart of
  Traefik). Header names that alias others (`X_Forwarded_For`) are dropped on every entrypoint:
  some backends (PHP, WSGI) read them as the real ones.
- The tunnel's entrypoint is always in Traefik (never published, routed only while the tunnel is
  on): turning the tunnel on no longer logs "EntryPoint doesn't exist" while Traefik restarts.
- **Cloudflare Tunnel** (tested with a real tunnel): cloudflared 2026.10.2 (2025.9.1 was no longer
  supported by Cloudflare; an older one is run again as pinned at the panel's start). A visitor over
  HTTPS at Cloudflare is told so to the service (`X-Forwarded-Proto: https`: links and HTTPS
  redirects stay right), and Let's Encrypt's HTTP challenges are answered through the tunnel (a
  domain reached only through it gets its certificate).
- A console command (`schedule:list`, `migrate`) run beside the panel no longer ends its open
  terminals or writes its route files: that is the server's work at start.

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

