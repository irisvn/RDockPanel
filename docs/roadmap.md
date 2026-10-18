# Roadmap

RDockPanel aims at EasyPanel's features, all free.

## Phase 1: server and Docker (done)

- Live server metrics, host facts.
- Containers (actions, live logs and stats, inspect, terminal), images (pull, remove, prune), volumes,
  networks.
- Protected panel objects, actions log and audit, fine-grained permissions.

## Phase 2: deploy apps to Docker Swarm (in progress)

Done so far: projects, services, domains, ports, mounts and deployments in the panel with their rules;
the Swarm API (services, tasks, networks, images); a development Swarm script; CI on a real Swarm; the
service spec and Traefik routes generated from the configuration, with images pinned (a registry digest, else a local image id);
deployments (Deploy, Start, Stop, Restart, Destroy, Rollback) run as jobs with a per-service lock,
convergence checks, automatic rollback and a live log; domains routed through Traefik (route files,
Republish, rewritten at boot, Traefik on each project's network), checked end to end: zero-downtime
image changes, HTTPS, isolated projects; environment variables and private registry credentials,
encrypted (RustCMS W8), kept with each deployment for rollback; the EasyPanel-like interface
(dashboard, service pages and tabs, React); environment placeholders, generated domains, file
mounts; builds from Git (https or ssh with a deploy key) or an inline Dockerfile, with Dockerfile or
Nixpacks, and deployment trigger URLs.

The installer (`install/install.sh`, the image `ghcr.io/irisvn/rdockpanel`, built for amd64 and
arm64): Swarm, MySQL, Traefik with Let's Encrypt, the panel, secrets kept across installs; tried on
Ubuntu 24.04 (install twice, reboot).

Next:
certificate status and drift checks (RustCMS W4). A deleted project's network is now removed
(Traefik detached first).

- Projects and services from a Docker image: replicas, resources, health checks, volumes, ports.
- Deployments as Swarm services: rolling updates (start-first), automatic rollback, deployment history
  and manual rollback, live deployment logs.
- Domains and HTTPS through Traefik (file provider, Let's Encrypt).
- Per-project networks.
- Encrypted environment variables and private registry credentials (needs RustCMS W8).
- A one-command installer (`install.sh`), the panel's Docker image, first-admin setup.

## Phase 3

- Builds from uploads, Railpack and buildpacks; GitHub webhooks with signatures, a GitHub token
  (builds from Git with Dockerfile or Nixpacks, an inline Dockerfile and trigger URLs are done).
- Volume backups (database backups to a local directory or S3, scheduled, with retention and
  restore, are done).
- Compose services from EasyPanel templates (Compose services and one-click templates are done).
- Notifications of Docker prunes and panel updates (Discord, Slack, Telegram, email and webhooks
  for deployments, backups and a full disk are done).
- Multiple servers (Swarm workers), placement.
- Multiple users per project, two-factor authentication.
- Public API with tokens, a CLI.

## Depends on RustCMS

| RustCMS | Used for | Status |
|---|---|---|
| 0.4.0: admin routes, server-sent events, hub, WebSocket, `*_FILE` secrets | Phase 1 | done |
| 0.4.1 W8: encrypted columns, key rotation | Phase 2 environment variables, registry credentials | released, next |
| 0.4.1 W4: scheduler | the deployment lease reaper (every minute); reconcile/drift, periodic prune, backups | released, in use |
| 0.5.0: React extension host (API v2) | the Phase 3 interface (dashboard, service pages, templates) | released, next |
| TOTP | two-factor authentication | planned |
