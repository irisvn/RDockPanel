# Roadmap

RDockPanel aims at EasyPanel's features, all free.

## Phase 1: server and Docker (done)

- Live server metrics, host facts.
- Containers (actions, live logs and stats, inspect, terminal), images (pull, remove, prune), volumes,
  networks.
- Protected panel objects, actions log and audit, fine-grained permissions.

## Phase 2: deploy apps to Docker Swarm (done)

- Projects and services from a Docker image: replicas, resources, health checks, volumes, ports, a
  network per project.
- Deployments as Swarm services, one at a time per service: pinned images, rolling updates
  (start-first), automatic and manual rollback, history and live logs.
- Domains and HTTPS through Traefik (route files, Let's Encrypt), checked end to end on a real Swarm:
  zero-downtime image changes, HTTPS, isolated projects.
- Encrypted environment variables and private registry credentials (RustCMS W8).
- The installer (`install/install.sh`, the image `ghcr.io/irisvn/rdockpanel` for amd64 and arm64):
  Swarm, MySQL, Traefik with Let's Encrypt, the panel; tried on Ubuntu 24.04 (install twice, reboot).

## Phase 3: EasyPanel's features (mostly done)

Done (feature by feature in [the EasyPanel comparison](easypanel-comparison.md)):

- The EasyPanel-like interface (React): the Dashboard as the panel's home, service pages and their
  tabs, Actions, Monitor, Settings, ⌘K.
- Databases (PostgreSQL, MySQL, MariaDB, MongoDB, Redis), EasyPanel templates, Create from Schema,
  Compose services.
- Builds from Git or an inline Dockerfile (Dockerfile, Nixpacks), trigger URLs, GitHub webhooks.
- Database and volume backups (local or S3, scheduled, retention, restore); notifications (Discord,
  Slack, Telegram, email, webhooks).
- Redirects, basic authentication, maintenance mode, scripts, notes, per-service CPU and memory,
  wildcard domains, custom Traefik middlewares, capabilities, sysctls and supplemental groups.

Next:

- Builds from uploads, Railpack and buildpacks; a GitHub token.
- A certificate resolver per domain; certificate status; drift checks (RustCMS W4).
- The panel's domain and Let's Encrypt email in Settings; a web database client.
- Notifications of Docker prunes and panel updates.
- RustCMS 0.6 for the React pages (its tabs, toasts and resource API).

## Later

- Multiple servers (Swarm workers), placement; Cloudflare Tunnel.
- Multiple users per project, two-factor authentication.
- A public API with tokens, a CLI.

## Depends on RustCMS

| RustCMS | Used for | Status |
|---|---|---|
| 0.4.0: admin routes, server-sent events, hub, WebSocket, `*_FILE` secrets | Phase 1 | done |
| 0.4.1 W8: encrypted columns, key rotation | Phase 2 environment variables, registry credentials | released, in use |
| 0.4.1 W4: scheduler | the deployment lease reaper (every minute); reconcile/drift, periodic prune, backups | released, in use |
| 0.5.0: React extension host (API v2) | the Phase 3 interface (dashboard, service pages, templates) | released, in use |
| 0.5.1 W9 part A: home page, sidebar order, hidden resources | an EasyPanel-like sidebar | released, in use |
| 0.6.0 W9 part B: resource API, toasts, UI kit 2 for React pages | replaces the panel's own tabs, toasts and resource calls | in progress |
| TOTP | two-factor authentication | planned |
