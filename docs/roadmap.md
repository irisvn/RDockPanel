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
encrypted (RustCMS W8), kept with each deployment for rollback.

Next: the installer, then
service logs and terminal, a projects overview, certificate status, and drift checks (RustCMS W4).
Deleting a project will also detach Traefik from its network and remove it.

- Projects and services from a Docker image: replicas, resources, health checks, volumes, ports.
- Deployments as Swarm services: rolling updates (start-first), automatic rollback, deployment history
  and manual rollback, live deployment logs.
- Domains and HTTPS through Traefik (file provider, Let's Encrypt).
- Per-project networks.
- Encrypted environment variables and private registry credentials (needs RustCMS W8).
- A one-command installer (`install.sh`), the panel's Docker image, first-admin setup.

## Phase 3

- Builds from GitHub/Git/uploads (Dockerfile, Nixpacks/Railpack, buildpacks), webhooks.
- Databases (PostgreSQL, MySQL, MariaDB, MongoDB, Redis) with scheduled backups to S3 and restore.
- Docker Compose, one-click templates.
- Notifications (Discord, Slack, Telegram, email).
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
