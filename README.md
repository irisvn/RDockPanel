# RDockPanel

**A free, self-hosted server panel in the spirit of [EasyPanel](https://easypanel.io), built in Rust.**
Watch your server, manage Docker, and (soon) deploy apps to Docker Swarm with domains and SSL. Every
feature is free: no project limits, no paid tiers.

[Tiếng Việt](README.vi.md) · [Docs](docs/) · [Roadmap](docs/roadmap.md)

> **Status: early development.** Phase 1 (server monitoring and Docker management) works. Deploying
> apps (Phase 2) is in progress. There is no installer yet: run it from source as below. Not for
> production use.

![Server dashboard](docs/images/server.png)

## Features

**Available now (Phase 1)**

- **Server**: live CPU, memory (with swap), network in/out and disk charts (2-second samples, the last
  5 minutes), plus host facts (OS, kernel, cores, uptime).
- **Containers**: list, search, start/stop/restart/pause/kill/remove; live logs (follow, stderr
  highlighted), live stats, inspect (secret-looking env values hidden), and a **terminal** in the
  browser.
- **Images**: pull (progress shown live), remove per tag, prune stopped containers and dangling images.
- **Volumes and networks**: create and remove (bridge or overlay).
- **Safety**: the panel's own containers, volumes and networks (labelled `rdockpanel.system=true`) are
  hidden and untouchable without the `system.manage` permission; prune skips them.
- **Accountability**: every change to Docker is recorded in an actions log and the audit log, with
  who did it.
- **Users and roles**: fine-grained permissions (`docker.view`, `docker.manage`, `docker.logs`,
  `docker.exec`, `server.view`, `system.manage`), built on RustCMS's roles.

**Coming next** (see the [roadmap](docs/roadmap.md))

- Phase 2: projects and services deployed to **Docker Swarm** from an image, rolling updates with
  rollback, domains and HTTPS through **Traefik**, a one-command installer.
- Phase 3: builds from Git, databases with backups, Compose, templates, notifications, multiple
  nodes, two-factor authentication, an API and a CLI.

| Containers | Live logs | Terminal |
|---|---|---|
| ![Containers](docs/images/containers.png) | ![Logs](docs/images/logs.png) | ![Terminal](docs/images/terminal.png) |

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
rustcms serve                                   # http://localhost:8000/admin
```

The panel talks to the local Docker daemon (`/var/run/docker.sock` or `DOCKER_HOST`): **it manages
the real containers on your machine**.

## Tests

```sh
export RUSTCMS_TEST_MYSQL=mysql://root:root@127.0.0.1:3306/mysql RUSTCMS_TEST_BACKENDS=mysql
cargo test                       # unit and feature tests against MySQL, Docker faked
PANEL_TEST_DOCKER=1 cargo test   # also against the real Docker engine (runs an alpine container)
```

CI runs formatting, `clippy -D warnings` and the tests on MySQL 8.4 on pushes to `main` and pull
requests (the real-Docker tests are not part of CI).

## How it is built

- **[RustCMS](https://github.com/irisvn/RustCMS-Core)**, a Laravel/Filament-like framework on axum and
  SeaORM: authentication, roles, audit log, job queue, the admin panel, server-sent events and
  WebSocket admin routes.
- **[bollard](https://github.com/fussybeaver/bollard)** for the Docker Engine API, **sysinfo** and
  `/proc` for host metrics, **MySQL 8.4** for the panel's data.
- The admin pages are scripts loaded by the RustCMS panel; the terminal uses
  [xterm.js](https://xtermjs.org) (vendored, MIT).

More in [docs/architecture.md](docs/architecture.md).

## Documentation

| | |
|---|---|
| [Architecture](docs/architecture.md) | modules, request flow, live updates, actions and jobs |
| [Permissions](docs/permissions.md) | who can see and do what |
| [Security](docs/security.md) | the Docker socket, protected objects, the terminal, CSP |
| [API](docs/api.md) | the panel's JSON, event-stream and WebSocket routes |
| [Development](docs/development.md) | environment, tests, conventions |
| [Roadmap](docs/roadmap.md) | phases and what they depend on |

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT),
at your option. xterm.js is MIT-licensed ([its license](resources/admin/js/rdockpanel/vendor/LICENSE-xterm)).
