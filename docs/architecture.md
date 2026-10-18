# Architecture

RDockPanel is a [RustCMS](https://github.com/irisvn/RustCMS-Core) application: one binary
(`rdockpanel`) that serves the admin panel, its JSON/event/WebSocket routes, and runs queue
workers. It follows the standard RustCMS layout (`rustcms make:*` generators).

```
src/
  bootstrap/        the app from its providers, then its API and admin routes
  providers/        auth (permissions), admin panel (pages, scripts, styles), app (services, jobs, boot)
  routes/admin.rs   the panel's own routes under /admin/api/x (AdminRoutes)
  http/controllers/ docker_controller (containers, images, volumes, networks, terminal), server_controller,
                    deploy_controller (a deployment's live log)
  services/         docker_service (+ bollard), system_stats_service, live_service, action_service;
                    deploy_rules_service (rules, [panel] config), deploy_service (spec and routes),
                    deployment_service (running deployments), traefik_service (route files)
  models/           action (the actions log); project, service, service_domain/port/mount, deployment
  jobs/             pull_image, run_deployment, publish_routes
  admin/pages/      Server, Containers, Images, Volumes, Networks (script pages)
  admin/resources/  Projects, Services, Domains, Ports, Mounts, Deployments (schema-driven resources)
  admin/slots/      the service page's aside (state, actions, live deployment log)
resources/admin/    the pages' JavaScript and CSS (+ vendored xterm.js)
scripts/            dev-swarm.sh (a development Swarm with Traefik)
tests/              feature tests (MySQL), a FakeDocker, real-engine tests behind PANEL_TEST_DOCKER and
                    PANEL_TEST_SWARM
```

## Services

| Service | Role |
|---|---|
| `Docker` (`docker_service.rs`) | Wraps the Docker Engine API (`DockerApi`: Bollard in production, `FakeDocker` in tests). Applies the panel's rules: hides and protects objects labelled `rdockpanel.system=true` unless the caller has `system.manage`, resolves every name to Docker's full id before acting (no path segments reach the Engine API), redacts secrets in `inspect`, ends terminal shells. |
| `SystemStats` (`system_stats_service.rs`) | Samples the host every 2 s and keeps 150 samples (5 min) in memory. Reads sysinfo, or the host's `/proc` mounted at `HOST_PROC` when the panel runs in a container. Nothing is stored in the database. |
| `Live` (`live_service.rs`) | Shared producers for live data through the framework's hub: one Docker stats stream per container and one host-sample feed for every viewer. Shapes log lines into events under 64 KiB and paces them. |
| `action_service` | The actions log: a row per Docker management operation (`queued`, `running`, `ok`, `failed`), audited when a user asked. Deployments are recorded in `deployments` instead. Stale rows (no update for 1 h) are marked failed at boot. |
| `Traefik` (`traefik_service.rs`) | Where Traefik's route files go and which Swarm service Traefik is (`[panel] traefik_dir`, `traefik_service`). Writes and removes route files, attaches Traefik to project networks (see [Routes](#routes-traefik)). |
| `deployment_service`, `deploy_service`, `deploy_rules_service` | Deployments: asking for one, running it, what a service becomes in Swarm and Traefik, the rules (see [Projects and services](#projects-and-services-phase-2-in-progress)). |

`Docker`, `SystemStats`, `Live` and `Traefik` are bound in `providers/app_service_provider.rs` and
read by handlers and jobs with `app.service::<T>()`. Tests bind fakes over them (`FakeDocker`, a
`Traefik` writing to a temporary directory).

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

## Projects and services (Phase 2, in progress)

Projects and services are RustCMS resources (forms, lists, relations, validation, audit come with the
framework). Their rules (names, hostnames, paths, ports, images, bind mounts) are pure functions in
`services/deploy_rules_service.rs`, with the `[panel]` configuration (`config/panel.toml`). A service
becomes the Swarm service `<project>_<service>`, labelled `rdockpanel.managed=true` with its project
and service: `Docker::managed_service` refuses any service of that name the panel did not deploy, and
Swarm updates go at the version just read, retried when Swarm moved it meanwhile.

`services/deploy_service.rs` turns a service into what it becomes, as pure functions:

- `Snapshot::of(...)` is the configuration one deployment applies (stored with it for history and
  rollback). Of the environment it keeps the variable names and a digest keyed with `APP_KEY`
  (HMAC-SHA256, `env_digest`), never a value; of the registry, the user. A secret written into a
  command or a health check is kept as typed. `Snapshot::hash()` (SHA-256 of its JSON, lists sorted)
  tells whether the configuration changed since the last deployment, a new environment value
  included.
- `pin_image` pins the configured image to what was just pulled: the digest the pull reported
  (`nginx:1.27@sha256:…`), else the repository's only registry digest (Docker Hub's names `nginx`,
  `library/nginx`, `docker.io/library/nginx` are one; several digests are ambiguous), else the local
  image id, with a warning that only that node has it.
- `desired(...)` gives the Swarm `ServiceSpec`: on the project network with the service's short name
  as alias, ownership labels on the service and its containers (the configuration hash on the service
  only, so a new domain or more replicas restart no task; the containers carry their task template's
  hash, `rdockpanel.template`), health check, limits and reservations, volumes
  (`rdp_<project>_<service>_<name>`), ports, start-first updates when zero downtime is on, the service
  has a health check and no host-mode port (stop-first otherwise), automatic rollback, 0 replicas when stopped. And the Traefik routes: one
  router per domain (HTTPS routers with an HTTP router redirecting to them on `[panel] https_port`,
  `certResolver` from `[panel] cert_resolver`), one Traefik service per container port, reached as
  `http://<project>_<service>:<port>`.

Snapshot tests (`src/services/snapshots/`, insta) pin the generated spec and routes; a Swarm test runs a
generated spec on a real engine.

### Secrets

A service's environment variables (`services.env`) and registry password
(`services.registry_password`) are RustCMS encrypted columns (`Encrypted<T>`, sealed with `APP_KEY`):
the form shows the variables' names and whether a password is set, never a value, and the audit log
never holds them. Asking for a deployment seals the environment it applies into `deployments.env`
(its own context), so a rollback applies the environment of then. The run opens it, puts it in the
Swarm spec (`Env`), and sends the service's registry credentials (as they are now) with the pull and
with the service's create or update, so every node can pull. Credentials belong to the registry of
the service's image (Docker Hub when the image names none): a run whose image is on another registry
(a rollback to an image since moved) pulls without them, and says so in its log. Destroy opens no
secret. After an `APP_KEY` rotation the digests differ but the environment does not: the pending
check then digests the deployed environment with today's key before telling a change. The task template's hash label counts the environment by its
keyed digest. A value sealed with a key that is gone (see `APP_PREVIOUS_KEYS` and
`rustcms key:reencrypt`, which knows all three columns) refuses the deployment with a clear error.

### Deployments

`services/deployment_service.rs` runs them:

1. **Request** (in the action's transaction): the deployment row is written `queued` with its snapshot,
   and the service's lock is taken with one conditional `UPDATE` (free, or its lease expired: an hour
   while queued, 5 minutes once running and not renewed), the service and project rows locked so a
   rename waits or goes first. Taken already: the action answers 409 `deployment_in_progress`; a
   deployment whose expired lock is taken over is marked `interrupted`. Then the `RunDeployment`
   job is dispatched (queue `deploy`, one attempt, 15 minutes). A rollback applies the earlier
   configuration and image under the service's names now.
2. **Run** (the job): it goes on only while the service's lock is still its own and unexpired
   (fencing by deployment id). It renews the lease, and stops if the lock is no longer its own, at the
   start, before the network and Traefik are set up, while pulling, before each create, update or
   removal of the service, and at each look at Swarm. Destroy removes the service. Otherwise: the bind mounts are checked again,
   the project network is ensured, the image is pulled and pinned (Deploy and Restart; Start and Stop
   reuse the image the service runs, if it exists: a Start of a service Docker does not have pulls,
   a Stop of one leaves nothing to do; Rollback uses the recorded one), the spec is created or updated (Restart bumps
   `force_update`), and the job waits until Swarm converged: an update's status (a stale one from the
   previous update is ignored; paused is a failure, rollback completed is `rolled_back`, the service
   then running or stopped as the restored spec says), else the tasks: as many in Swarm's `running` state (which,
   with a health check, a task reaches once healthy) as wanted, all of the new task template (its hash is the containers' `rdockpanel.template` label,
   so a restart or a new command is told apart from the old tasks), or, when stopped, every task
   ended. Steps go to the deployment's log, saved about every second.
3. **Finish** (one transaction): the deployment's status and pinned image, the service's status, and
   the lock released only if still its own.

At boot, then every minute (the RustCMS scheduler's `reap-deployments`, run in the process),
deployments whose lease expired (their run died, or its job timed out) become `interrupted` and free
their service.

The service page's aside (`ServiceLiveSlot`, `service-live.js`) shows the state, pending changes, the
action buttons and the latest deployment, its log followed live over SSE.

### Routes (Traefik)

`services/traefik_service.rs`. Traefik reads one file per service, `<project>_<service>.yml`, from
`[panel] traefik_dir` (its file provider; `storage/traefik/dynamic` in development). The panel
writes it whole (a hidden temporary file, then renamed) after each deployment, from the deployment
that made the service what it is (its configuration hash): a new domain is routed once deployed, a
failed deployment leaves the routes of what still runs, Stop and Destroy remove the file before the
tasks go (if that fails, the deployment goes on with a warning in its log).
Whether the write worked is the deployment's `published`; the service's **Republish routes** action
(a `deploy.publish` job) writes it again. At boot every file is written again from the database, and
files of services that no longer exist are removed.

Traefik reaches a service as `http://<project>_<service>:<port>` on the project's network: before
the first deployment with a domain, the panel attaches `[panel] traefik_service` (`rdp-traefik`) to
`rdp_<project>` (Swarm restarts Traefik then: a few seconds without routing); when it cannot, the
deployment fails rather than publish domains that lead nowhere. Writes are serialized (one lock in
the panel's process, workers included), each reading the database at that moment, so the last one
written is always the latest state. Projects share no network, so a project's services cannot call
another's directly (domains and published ports remain public). Deleting a project does not yet detach Traefik from its network nor remove the network
(see the [roadmap](roadmap.md)).

## Data

MySQL 8.4 (`docker-compose.yml` for development). Besides the framework's tables (users, roles,
sessions, audit log, jobs, schedule slots, …), the panel adds `actions`, then for Phase 2 `projects`,
`services`, `service_domains`, `service_ports`, `service_mounts` and `deployments` (each with its
configuration snapshot, pinned image, status and log).
