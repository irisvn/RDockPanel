# RDockPanel

**A free, self-hosted server panel in the spirit of [EasyPanel](https://easypanel.io), built in Rust.**
Watch your server, manage Docker, and deploy apps to Docker Swarm with domains and HTTPS. Every
feature is free: no project limits, no paid tiers.

[Tiếng Việt](README.vi.md) · [Docs](docs/) · [Roadmap](docs/roadmap.md)

> **Status: early development.** Server monitoring, Docker management, apps and databases on
> Docker Swarm with domains and HTTPS, builds from Git or a Dockerfile, EasyPanel templates,
> Compose services, database and volume backups, notifications, and a one-command installer work.
> Try it on a test server first.

![Dashboard](docs/images/dashboard.png)

## Features

**The interface, like EasyPanel's**

- `/admin` opens the **Dashboard**: server tiles, projects with their service cards, **+ Service**
  and **Templates**. The sidebar has Dashboard, Actions, Settings and the Docker pages.
- A **service page** has its header (status, Deploy, Start/Stop, Restart, Open, Logs, Shell) and tabs:
  Overview, Source, Deployments, Environment, Domains, Redirects, Security, Maintenance, Scripts,
  Storage, Backups (databases), Advanced.
- **Actions** lists deployments, backups and Docker actions; **Settings** holds storage providers and
  notification channels; **⌘K** finds a project or a service. Light by default, with a Light / Dark /
  System switch per user; in English and Vietnamese.
- Every page follows one set of design rules ([docs/ui-guidelines.md](docs/ui-guidelines.md)): the
  same button sizes, extra row actions in a ⋯ menu, the same status words and colors, the panel's own dialogs,
  tables that work on a phone.

**Server and Docker**


- **Server metrics** on the Dashboard: live CPU, memory, disk and network in/out, with their recent
  history.
- **Containers**: list, search, start/stop/restart/pause/kill/remove; live logs (follow, stderr
  highlighted), live stats, inspect (secret-looking env values hidden), and a **terminal** in the
  browser.
- **Images**: pull (progress shown live), remove per tag, prune stopped containers and dangling images.
- **Volumes and networks**: create and remove (bridge or overlay).
- **Safety**: the panel's own containers, volumes and networks (labelled `rdockpanel.system=true`) are
  hidden and untouchable without the `system.manage` permission; prune skips them.
- **Accountability**: every Docker management operation (containers, images, volumes, networks,
  terminals) is recorded in an actions log and the audit log, with who did it; deployments keep their
  own history (who asked, when, the log).
- **Users and roles**: fine-grained permissions (`docker.view`, `docker.manage`, `docker.logs`,
  `docker.exec`, `server.view`, `system.manage`, plus one set per resource such as `services.deploy`),
  built on RustCMS's roles.

**Apps on Docker Swarm**

- **Projects and services**: a service runs a Docker image on **Docker Swarm** with replicas, CPU and
  memory limits, a health check, named volumes, published ports and a command. Each project has its
  own network: a project's services cannot call another project's services directly (what is public,
  through a domain or a published port, stays public).
- **Deployments**: Deploy, Start, Stop, Restart and Destroy from the service page, one at a time per
  service, with a live log; Rollback from an earlier deployment ("Roll back to this"); Force rebuild
  (no build cache, the image pulled again, every task replaced); the Overview's recent errors. Images are
  pinned to their registry digest (a local-only image to its id, with a warning). Updates are rolling
  (start-first when zero downtime is on, the service has a health check and publishes no host-mode
  port; stop-first otherwise) and Swarm rolls back a failed one. Every deployment keeps its
  configuration, for history and rollback, and the page shows changes not deployed yet.
- **Domains and HTTPS** through **Traefik**: domains and paths per service; a domain with HTTPS on
  redirects its HTTP to HTTPS; wildcard domains (`*.example.com`, for administrators) and custom
  Traefik middlewares; redirects (regular expressions), HTTP basic authentication, and a maintenance page.
  A **Domains** page lists every service's domains, where they lead and whether they are served.
  Domains, redirects, basic authentication and maintenance **apply when saved** (no Deploy, the
  running containers untouched); image, environment, ports, mounts and resources need a Deploy.
  A certificate resolver per domain (Settings → Certificates: Let's Encrypt over HTTP or over DNS with
  a Cloudflare, DigitalOcean, Hetzner, Vultr or Linode token), applied to Traefik without SSH; or a
  certificate of your own, uploaded (checked, its key sealed, warned of before it expires).
- **Settings → Version**: what the panel runs (and with: Core, Traefik, MySQL, Docker, the OS), whether a
  newer release is out (checked every 6 hours, also said on the Dashboard), what changed, the installer
  command that updates it (the panel never updates itself), its history, and its old versions removed.
- **Settings → Panel** has EasyPanel's menus: the panel's logs and restart; Refresh IP and Reboot
  for the server; Traefik's logs and restart; Docker prunes that never touch the services' data.
- **Databases**: PostgreSQL, MySQL, MariaDB, MongoDB and Redis in one click (a name is enough:
  credentials generated, data on a volume, health-checked), with their connection URL to copy.
- **EasyPanel templates**: the 800+ templates of easypanel.io in one click (searched, filled in by
  their own form, deployed with their databases), fetched from their repository when you open the
  catalog; or paste a template's output ("Create from Schema").
- **Builds**: an app from a Git repository (https, ssh with a generated deploy key, or a GitHub token of
  Settings → Git given to its project, its repositories picked from a list; a branch, tag
  or commit; a sub-directory), built with its Dockerfile or Nixpacks, or from a Dockerfile written in
  the panel; the build's log is the deployment's. A deployment trigger URL deploys from a CI, and a
  signed GitHub webhook deploys on each push (auto deploy).
- **Compose services**: a Compose file (written in the panel or from Git) run with `docker compose`
  on the project's network, its domains leading to one of its services.
- **Backups**: scheduled dumps (cron) of PostgreSQL, MySQL, MariaDB and MongoDB, and archives of
  volumes, to a local directory or an S3-compatible bucket, with retention, a log per run, "Run now"
  and Restore.
- **Project members** (viewer, developer, admin per project) and a **public API** with scoped tokens
  (Settings → API tokens) and its command line, `rdp`.
- **Notifications**: Discord, Slack, Telegram, email (SMTP) and webhooks; each channel picks its
  events (deployments, backups, a full disk, a failing service and its return, Traefik down, expiring
  certificates, a newer panel, Docker prunes, a sign-in from a new address), with "Send test notification".
- **Panel in Settings**: its domain, Let's Encrypt email and apps' domain, checked (DNS, names in
  use) with the installer command that applies them.
- **Traefik in Settings**: its state, a restart, its live logs, and middlewares of your own
  (`custom-<name>`, headers, rate limits, IP allow lists…) for domains, all without SSH.
- **Cloudflare Tunnel** (Settings): cloudflared run with your tunnel's token (kept encrypted, given
  as a Swarm secret), so domains are reached through Cloudflare without opening ports 80 and 443.
- **Branding** (Settings): the panel's name, primary color, logos (light and dark) and favicon,
  on every page and the sign-in page, without a restart.
- **Roles page**: permissions in areas (Apps, Docker, Server, System) with labels and descriptions,
  a search, and presets (apps viewer, developer, administrator; monitor) to start a role from.
- **Per service**: CPU and memory charts, notes for the team, scripts (one-off commands in the running
  container, their output shown), and, for administrators, capabilities, sysctls and supplemental
  groups.
- **Secrets**: environment variables and private registry credentials, encrypted with the app key;
  edited as `.env` text; values are shown only with the `services.secrets` permission (each read is
  audited). A rollback brings back the environment of then.

**Coming next** (see the [roadmap](docs/roadmap.md))

- Uploads as a source, Railpack and buildpacks; a web database client.
- Later: multiple nodes, two-factor authentication.

| A service | Templates | + Service |
|---|---|---|
| ![A service](docs/images/service.png) | ![Templates](docs/images/templates.png) | ![+ Service](docs/images/new-service.png) |
| Deployments | Domains | A database |
| ![Deployments](docs/images/deployments.png) | ![Domains](docs/images/domains.png) | ![A database](docs/images/database.png) |
| Actions | Settings | Containers |
| ![Actions](docs/images/actions.png) | ![Settings](docs/images/settings.png) | ![Containers](docs/images/containers.png) |
| Live logs | Terminal |   |
| ![Live logs](docs/images/logs.png) | ![Terminal](docs/images/terminal.png) |   |

## Install

On a fresh Ubuntu or Debian server (amd64 or arm64):

```sh
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- --admin-email you@example.com
```

Add `--domain panel.example.com --email you@example.com` for the panel on HTTPS (Let's Encrypt)
and `--apps-domain apps.example.com` for generated app domains. See [docs/install.md](docs/install.md).

## Quick start (development)

You need Rust (stable), Docker (Docker Desktop or OrbStack on macOS) and the
[`rustcms`](https://github.com/irisvn/RustCMS) CLI.

```sh
git clone https://github.com/irisvn/rdockpanel && cd rdockpanel
cp .env.example .env && rustcms key:generate    # APP_KEY
docker compose up -d                            # MySQL 8.4 on 127.0.0.1:3306
rustcms migrate
rustcms permission:sync
rustcms make:user --email admin@example.com --name Admin --super-admin
sh scripts/dev-swarm.sh                         # a one-node Swarm, Traefik on :8080 and :8443
rustcms serve                                   # http://localhost:8000/admin
```

Open the panel at `localhost` (the `APP_URL` of `.env`), not `127.0.0.1`: sign-in checks the origin.
A service with the domain `app.localhost` is then at http://app.localhost:8080 (and at
https://app.localhost:8443 with Traefik's self-signed certificate, when the domain has HTTPS on).

The panel talks to the local Docker daemon (`/var/run/docker.sock` or `DOCKER_HOST`): **it manages
the real containers on your machine**. `dev-swarm.sh` turns it into a Swarm manager
(`docker swarm leave --force` undoes it).

## Tests

```sh
export RUSTCMS_TEST_MYSQL=mysql://root:root@127.0.0.1:3306/mysql RUSTCMS_TEST_BACKENDS=mysql
cargo test                       # unit and feature tests against MySQL, Docker faked
PANEL_TEST_DOCKER=1 cargo test   # also against the real Docker engine (runs an alpine container)
PANEL_TEST_SWARM=1 cargo test    # also deploys on the real Swarm and routes through Traefik
                                 # (after scripts/dev-swarm.sh)
```

CI runs formatting, `clippy -D warnings` and the tests on MySQL 8.4 on pushes to `main` and pull
requests, and a second job on a real Docker engine and Swarm with Traefik. That job deploys, makes
rolling updates under load, and checks HTTPS and project isolation.

## How it is built

- **[RustCMS](https://github.com/irisvn/RustCMS-Core)**, a Laravel/Filament-like framework on axum and
  SeaORM: authentication, roles, audit log, job queue, the admin panel, server-sent events and
  WebSocket admin routes.
- **[bollard](https://github.com/fussybeaver/bollard)** for the Docker Engine API (Swarm services,
  tasks, networks), **sysinfo** and `/proc` for host metrics, **MySQL 8.4** for the panel's data.
- **Docker Swarm** runs the apps; **[Traefik](https://traefik.io)** v3 routes their domains, from
  files the panel writes.
- The apps interface is React pages on RustCMS's extension host (the built files are committed:
  Node is needed only to change them), the Docker pages included; the terminal uses
  [xterm.js](https://xtermjs.org) (vendored, MIT).

More in [docs/architecture.md](docs/architecture.md).

## Documentation

| | |
|---|---|
| [Architecture](docs/architecture.md) | modules, request flow, live updates, actions and jobs |
| [Permissions](docs/permissions.md) | who can see and do what |
| [Security](docs/security.md) | the Docker socket, protected objects, the terminal, CSP |
| [API](docs/api.md) | the panel's JSON, event-stream and WebSocket routes |
| [Install](docs/install.md) | the installer, upgrades, backups |
| [EasyPanel comparison](docs/easypanel-comparison.md) | what RDockPanel has of EasyPanel, feature by feature |
| [Development](docs/development.md) | environment, tests, conventions |
| [UI guidelines](docs/ui-guidelines.md) | the design rules every page follows |
| [Roadmap](docs/roadmap.md) | phases and what they depend on |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT),
at your option. xterm.js is MIT-licensed ([its license](resources/admin/js/rdockpanel/vendor/LICENSE-xterm)).
