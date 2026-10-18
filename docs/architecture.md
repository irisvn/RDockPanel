# Architecture

RDockPanel is a [RustCMS](https://github.com/irisvn/RustCMS-Core) application: one binary
(`rdockpanel`) that serves the admin panel, its JSON/event/WebSocket routes, and runs queue
workers. It follows the standard RustCMS layout (`rustcms make:*` generators).

```
src/
  bootstrap/        the app from its providers, then its API and admin routes
  providers/        auth (permissions), admin panel (pages, scripts, styles), app (services, jobs, boot)
  routes/admin.rs   the panel's own routes under /admin/api/x (AdminRoutes)
  http/controllers/ docker_controller (containers, images, volumes, networks, terminal), server_controller
  services/         docker_service (+ bollard), system_stats_service, live_service, action_service
  models/           action (the actions log)
  jobs/             pull_image
  admin/pages/      Server, Containers, Images, Volumes, Networks (script pages)
resources/admin/    the pages' JavaScript and CSS (+ vendored xterm.js)
tests/              feature tests (MySQL), a FakeDocker, real-engine tests behind PANEL_TEST_DOCKER
```

## Services

| Service | Role |
|---|---|
| `Docker` (`docker_service.rs`) | Wraps the Docker Engine API (`DockerApi`: Bollard in production, `FakeDocker` in tests). Applies the panel's rules: hides and protects objects labelled `rdockpanel.system=true` unless the caller has `system.manage`, resolves every name to Docker's full id before acting (no path segments reach the Engine API), redacts secrets in `inspect`, ends terminal shells. |
| `SystemStats` (`system_stats_service.rs`) | Samples the host every 2 s and keeps 150 samples (5 min) in memory. Reads sysinfo, or the host's `/proc` mounted at `HOST_PROC` when the panel runs in a container. Nothing is stored in the database. |
| `Live` (`live_service.rs`) | Shared producers for live data through the framework's hub: one Docker stats stream per container and one host-sample feed for every viewer. Shapes log lines into events under 64 KiB and paces them. |
| `action_service` | The actions log: a row per Docker operation (`queued`, `running`, `ok`, `failed`), audited when a user asked. Stale rows (no update for 1 h) are marked failed at boot. |

`Docker`, `SystemStats` and `Live` are bound in `providers/app_service_provider.rs` and read by
handlers with `app.service::<T>()`.

## A request

1. The browser loads `/admin` (the RustCMS SPA). The panel's scripts register their pages
   (`window.rustcms.registerPage("rdockpanel/containers", …)`).
2. A page calls `/admin/api/x/docker/...` with the session cookie (and the CSRF token on writes).
3. `AdminRoutes` checks the route's permission **before** any extractor runs (401/403).
4. The handler asks `Docker` for the caller's scope (`system.manage` or not), acts, and records an
   action and an audit entry for changes.

## Live updates

- **Server-sent events** (`AdminRoutes::events`): host samples (`/server/live`), container stats and
  log follow. Each stream rechecks the session and, per container, whether the viewer may still see it
  (`target`), every 30 s.
- **WebSocket** (`AdminRoutes::socket`): the terminal. Keys travel as binary messages, resizes as
  `{"type":"resize"}` and are re-authorized when they run.
- The hub is process-local: the panel runs as one replica.

## The terminal

`docker exec` of `bash` (or `sh`) on a TTY, its processes marked with `RDOCKPANEL_SESSION=<token>`.
When the socket ends (closed tab, revoked access, idle 30 min, shutdown) the panel hangs up and then
kills every process in the shell's session, found through `/proc`. A terminal cut by a panel stop is
ended at the next boot (its token is kept in its action row). Each session is an action
(`container.exec`) with its duration; what is typed is never recorded.

## Jobs

The RustCMS queue (database-backed). Workers in `serve` take jobs from `default`, `docker` (image
pulls) and `deploy` (Phase 2), configured in `config/queue.toml`.

## Data

MySQL 8.4 (`docker-compose.yml` for development). Besides the framework's tables (users, roles,
sessions, audit log, jobs, …), the panel adds `actions`. Phase 2 adds projects, services and
deployments.
