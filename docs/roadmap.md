# Roadmap

Every feature free: no project limits, no paid tiers.

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

## Phase 3: apps, builds and Settings (mostly done)

Done:

- The interface (React): the Dashboard as the panel's home, service pages and their
  tabs, Actions, Settings (tabbed), server metrics on the Dashboard, ⌘K.
- Databases (PostgreSQL, MySQL, MariaDB, MongoDB, Redis), 800+ EasyPanel templates, Create from Schema,
  Compose services.
- Builds from Git (the repository's Dockerfile) or an inline Dockerfile, trigger URLs, GitHub webhooks;
  GitHub tokens (Settings → Git) given to projects, their repositories picked from a list.
- Database and volume backups (local or S3, scheduled, retention, restore); notifications (Discord,
  Slack, Telegram, email, webhooks).
- Redirects, basic authentication, maintenance mode, scripts, notes, per-service CPU and memory,
  wildcard domains, custom Traefik middlewares, capabilities, sysctls and supplemental groups.
- Certificate resolvers per domain (Let's Encrypt over HTTP or DNS), applied to Traefik from the panel;
  uploaded certificates with expiry warnings; Traefik in Settings (state, restart, logs, custom
  middlewares).
- Cloudflare Tunnel (Settings): cloudflared with the tunnel's token as a Swarm secret; every domain
  and the panel also served on Traefik's `tunnel` entrypoint (plain HTTP, Cloudflare holds TLS).
- Branding in Settings (RustCMS 0.7.0 runtime branding); the Roles page's areas, labels and presets.
- Project members and the public API with scoped tokens (`rdp`); Settings → Version (update,
  history, old versions removed); the public homepage at `/`.

Next:

- Builds from uploads.
- Certificate status of the resolvers; drift checks (RustCMS W4).
- A web database client; the panel's domain applied from Settings (today: checked there, applied by the installer).

## Later

- Multiple servers (Swarm workers), placement.
- Two-factor authentication.

## Depends on RustCMS

| RustCMS | Used for | Status |
|---|---|---|
| 0.4.0: admin routes, server-sent events, hub, WebSocket, `*_FILE` secrets | Phase 1 | done |
| 0.4.1 W8: encrypted columns, key rotation | Phase 2 environment variables, registry credentials | released, in use |
| 0.4.1 W4: scheduler | the deployment lease reaper (every minute); reconcile/drift, periodic prune, backups | released, in use |
| 0.5.0: React extension host (API v2) | the Phase 3 interface (dashboard, service pages, templates) | released, in use |
| 0.5.1 W9 part A: home page, sidebar order, hidden resources | the panel's sidebar | released, in use |
| 0.6.0 W9 part B: resource API, toasts, UI kit 2, page titles, any-of page permissions | the kit's tabs, page titles on sub-paths, Settings for either permission, icons | released, in use |
| 0.6.1: a light/dark switch per user | the panel's sidebar | released, in use |
| 0.7.0: runtime branding | Settings → Branding | released, in use |
| 0.8.x: `migrate --allow-newer` | an older panel starting on a newer database (Settings → Version's rollback) | released, in use |
| 0.8.2: pages remounted after an action | Stop/Start from a service's tab without the error page | released, in use |
| 0.10.3: media, rich text v2, editor layouts, action options (0.9–0.10) | security fixes; the panel keeps its look (success and warning stay neutral) | released, in use (the pin) |
| TOTP | two-factor authentication | planned |
