# Docker Compose

A Compose service runs a whole `docker-compose.yml` (several containers) as one service of a project. Use it for apps shipped as a Compose file; for everything else, prefer an app service, which runs on Swarm with rolling updates and rollback.

## Who can use it

Only the panel's administrators (`system.manage`) create and change Compose services: a Compose file can run anything on the server (host folders, privileged containers). Project members can see them, but not change, deploy or open a terminal in them.

## Create one

1. On a project, press **Add service** and choose **Compose**.
2. Type a name and write the **Compose file** in the editor:

   ```yaml
   services:
     web:
       image: ghcr.io/traefik/whoami:latest
   ```

3. Press **Create**, then **Deploy**.

**From Git** instead: on the **Source** tab, choose **Git repository**, fill in the **Repository URL**, the **Branch, tag or commit**, the **Build path** (the Compose project's folder) and the **Compose file** (`docker-compose.yml` by default). The repository is cloned once, then fetched at each Deploy; files it does not track (such as `./data`) stay.

## How it runs

- The panel runs it with `docker compose` on the panel's own server, not as a Swarm service.
- Each container joins the project's network, so the project's other services reach it as `<project>_<service>_<container>`.
- The service's [environment](/apps/environment) becomes the Compose project's `.env` file: use `${VAR}` in the Compose file.

| Button | Runs |
|---|---|
| **Deploy** | `docker compose up -d --build --remove-orphans` |
| **Start** | `docker compose up -d` |
| **Stop** | `docker compose stop` |
| **Restart** | `docker compose restart` |
| **Remove from Docker** | `docker compose down --remove-orphans` (named volumes are kept) |

## Domains

Add a domain on the **Domains** tab as for any service, and set **Compose service**: the service of the Compose file it leads to (such as `web`). Unlike other services, a Compose service's domains **apply on its next Deploy**.

## Limits

- No **Rollback**: deploy again with the file you want.
- No **Scripts** tab; no choice of node (it always runs on the panel's server).
- `container_name`, `ports` and `network_mode` in the file are warned about: they can clash with other services. Prefer domains to published ports.
- The memory check before a deploy counts the file's services; a file from a repository is not known before it runs.

::: tip Good to know
Templates that use Compose create Compose services, so they need `system.manage` too.
:::
