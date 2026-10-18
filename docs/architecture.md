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
resources/admin/    src/: the React apps interface (built to js/, css/); js/: the Docker and Server pages'
                    plain JavaScript (+ vendored xterm.js), css/
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

### EasyPanel templates

`services/template_service.rs`, `services/template_engine.rs`, `templates_controller.rs`. The
catalog (`list.json`), each template's `meta.yaml` and `index.ts`, and logos are fetched from
`[panel] templates_url` (EasyPanel's repository, raw files) when the Templates dialog opens, and
kept six hours; nothing is bundled (the repository has no license). Logos go through the panel
(`/apps/templates/{slug}/logo`): its CSP loads images from itself only.

A template runs as EasyPanel runs it: its form is its JSON Schema (defaults filled in); its
TypeScript generator (at most 256 KB), types stripped by oxc, runs in QuickJS with a 64 MB, 3 s
budget, in a child process of the panel's own binary (`rdockpanel __rdockpanel-run-template`: CPU
and, on Linux, memory limits, killed after 15 s; a crash takes the child down, never the panel),
and nothing to import but `~templates-utils` (`randomString`, `randomPassword`, `bcryptHash`), its `./meta` and
`crypto` (`randomBytes`, `randomUUID`; also `require("crypto")` and `Buffer.from` for older
templates); no file system, no network. Its `{services: [{type, data}]}` output becomes
services through `creation_service` (one transaction: all or none): apps from an image (env as
`.env` text, quoted and multi-line values as EasyPanel reads them, placeholders resolved at deploy, `$(EASYPANEL_DOMAIN)` domains as generated domains,
volumes, files, ports, command, replicas, zero downtime) and databases (their options), databases
first. What cannot be done is said, never dropped silently: host directories, redirects, resources
and registry credentials are left out with a warning; what a service needs to be safe or to run
(basic authentication, Linux capabilities, sysctls) is created but refused for "Deploy now"; Git, GitHub and
Dockerfile sources become builds (Dockerfile or Nixpacks; other builders refuse the template), and
Compose services become Compose services (a file of a repository, or inline; their domains name
their container; only for `system.manage`). The same output pasted
is "Create from Schema". Of the 843 templates of October 2026, 829 run; the others build from
source or use Compose (`PANEL_TEST_TEMPLATES=1` counts again).

### Databases

A database (`services.kind`: `postgres`, `mysql`, `mariadb`, `mongo`, `redis`) is an app service the
panel configures when it is created (`services/creation_service.rs`, `services/database_service.rs`):
the engine's image (`postgres:17`, `mysql:9`, `mariadb:11`, `mongo:8`, `redis:7` by default), a
`data` volume at the engine's data path, its credentials in the environment (sealed; passwords
generated: 24 letters and digits), a health check of the engine (`pg_isready`, `mysqladmin ping`,
…; every 5 s, 20 tries), one replica, stop-first updates (zero downtime is refused: two tasks
never share its data). Redis takes its password from the
environment (`--requirepass "$REDIS_PASSWORD"`), never from its arguments. So it deploys, rolls back
and logs like any service; the Credentials tab reads (an audited read, `services.secrets`) the
user, password and internal URL
(`postgres://…@<project>_<service>:5432/<db>`, …) back from its environment. A database's
credentials apply when its volume is first initialized: changing them later does not change the
database's own.

### Placeholders, generated domains, files

- **Placeholders** (`services/placeholders.rs`), as EasyPanel's templates write them, are replaced
  in the environment at each deployment: `$(PROJECT_NAME)`, `$(SERVICE_NAME)`, `$(PRIMARY_DOMAIN)`
  (the first domain added, else the generated one), `$(EASYPANEL_DOMAIN)` (the generated domain),
  `$(EASYPANEL_HOST)` (`[panel] service_domain`) and `$(EASYPANEL_HOST_IP)` (`[panel] public_ip`).
  An unknown or unset one is left as written.
- **Generated domains**: `<project>-<service>.<[panel] service_domain>` (a wildcard DNS record to
  the server; development: `localhost`), with a short hash of both names when the project's name has
  a dash or the label would pass 63 characters (never the same for two services). The Domains tab
  offers it; it is then a domain like any. The task template's hash counts the environment as
  resolved (keyed digest), so a placeholder resolving differently replaces the tasks; a change of
  `[panel] service_domain` or `public_ip` alone is not shown as pending.
- **File mounts** (`kind = file`) keep their content sealed with `APP_KEY`
  (`service_mounts.sealed_content`: templates put passwords in files); each deployment seals the
  files it applies (`deployments.files`, for rollback) and its configuration keeps only their keyed
  digests. They become Swarm configs named `rdp_<hash of project, service, path, digest>`, labelled
  `rdockpanel.config-of=<project>_<service>` and mounted read-only. A new content is a new config; a
  successful deployment removes the service's configs it no longer uses, Destroy all of them.

### Builds

An app's `source` is `image` (pulled), `git` or `dockerfile` (built by `build_service`). A Git
source is an https or ssh URL (never a local path or another transport), a branch, tag or commit
(a commit: the whole history, then that commit), a path in the repository and a builder: the
repository's Dockerfile (`dockerfile_path`), or Nixpacks writing one (`nixpacks build --out`). A
private repository uses the service's deploy key (ed25519, made by `ssh-keygen`, its private half
sealed like the environment, its public half shown to add to the repository). The repository is
cloned (`--depth 1`) into a temporary directory removed afterwards, packed as a tar (no `.git`,
nothing its `.dockerignore` lists, at most 1 GB) and built by Docker with BuildKit (`build_image`,
as `docker build`: `RUN --mount` works) as `rdp/<project>_<service>:d<deployment>`, on
this node (one node: nothing is pushed). Every step goes to the deployment's log, and the deployment's lock is renewed every 30 s
while a step runs (a lost lock stops the build). Deploy builds;
Start, Stop, Restart and Scale keep the image running (a first Start builds); Rollback runs the
image of then.

A service's trigger URL (`/api/v1/deploy/{id}?token={token}`) deploys it without a session: the token
(40 characters) is shown once, only its SHA-256 is stored and compared in constant time; a new
one replaces it. Unknown, wrong or malformed answers 404 the same; a deployment under way, 409.

### Redirects and basic authentication

As EasyPanel's Redirects and Security tabs. A service's redirects (`service_redirects`: a regular
expression over the request's URL, its replacement, permanent or not, on or off) and the users of
its HTTP basic authentication (`service_basic_auth`: bcrypt hashes, never shown) become Traefik
middlewares (`redirectRegex`, then `basicAuth`) on every router of its domains. They are part of
the deployment's snapshot (users as a digest of their hash: the hashes are read from their table
when the routes are written, never kept in a deployment) and apply on the next deployment: the
routes written again (a republish, a restart) keep the users as deployed, a changed password
signs in from the next deployment.
Regular expressions are checked as Go's (inline flags `i`, `m`, `s`, `U` only), and no value
written to Traefik's files may contain `{{` or `}}` (Traefik reads them as Go templates). EasyPanel templates' `basicAuth` and `redirects` are created with the service.

### Compose services

As EasyPanel's. A service of kind `compose` (made by the panel's administrators only:
`system.manage`, since a Compose file can run anything on the host) is the containers of a Compose
file, written in the panel or a file of its Git repository (cloned once into its directory, then
fetched: files the repository does not track, such as `./data`, stay). They run with `docker
compose` (the Docker CLI's plugin, in the panel's image) on this node, not as Swarm services, from
`compose_dir/<project>_<service>` (`/var/lib/rdockpanel/compose`, the same path on the host and in
the panel, since the daemon reads the file's relative paths on the host). The panel writes the file
again as `.rdockpanel-compose.yml`: each container also joins the project's network with the alias
`<project>_<service>_<container>` (what a domain naming that container leads to) and carries the
panel's labels; `container_name`, `ports` and `network_mode` are warned about. The service's
environment (placeholders resolved) is the Compose project's `.env` (`0600`), read by `${VAR}`.
Deploy runs `up -d --build --remove-orphans` (`create` for a stopped service), Start `up -d`,
Stop `stop`, Restart `restart`, Destroy `down --remove-orphans` (named volumes kept); Rollback is
refused. Stop, Start, Restart and Destroy act where it was last deployed (recorded), and a Deploy
from elsewhere takes the old containers down first. The repository is fetched without `--force`
(tracked files reset, a commit that would overwrite an untracked file fails the deployment). The
deployment's lock is renewed while `docker compose` runs. A domain must name a service of the
file.

### Database and volume backups

As EasyPanel's. A **storage provider** (Settings) is a local directory (absolute; in the
installed panel under `/var/lib/rdockpanel/backups`, a host directory) or an S3-compatible bucket
(requests signed by `rusty-s3`, sent by `reqwest`; its secret key sealed, context
`storage_providers.secret_key`). A database service (PostgreSQL, MySQL, MariaDB, MongoDB; Redis
keeps its append-only file) has **backup configurations**: a database, a five-field cron schedule
(UTC), a provider, a path (folders of letters, digits, `.`, `_`, `-`) and a retention.

A configuration may back up a **volume** of any service instead (its path in the container): a run
then runs `tar -czf - -C <path> .` in the service's running container (so the image needs `tar`),
writes `….tar.gz`, and a restore empties the directory and feeds an archive (read whole first
as a tar of relative paths) to `tar -xzf - -C <path>` (stop what writes there first). The volume is checked in the
running container itself: the service's named volume at that path, nothing mounted inside it.

A run (`backup_runs`, the `backup.run` job on the `backup` queue, one attempt, up to 3 hours) has
its file, provider and retention fixed when it is asked for (a later edit changes later runs). One
run of a service at a time (claimed with the service's row locked; another one running fails it).
A running run beats every 30 s; one silent for 150 s (its process stopped) has its command killed
in its container (the run records the container and the mark its processes carry) and is marked
failed, by the every-minute tick, at boot and before another run starts; a run that finds itself
taken for dead stops. It finds the service's running task and runs the engine's own dump in its container (`docker exec`, no TTY):
`pg_dump --clean --if-exists`, `mysqldump`/`mariadb-dump` as root with `--single-transaction`,
`mongodump --archive --gzip`. The credentials are the container's own environment (the panel
passes none); the database is `$1` of `sh -c` (never parsed). The panel gzips SQL dumps as they
come (scratch files in a private `0700` directory), writes
`<path>/<project>_<service>.<database>.<YYYYmmddTHHMMSSZ>.r<run>.sql.gz` (`.archive.gz` for MongoDB),
then deletes the oldest files of that service's database in that path beyond the retention (other
files are left alone; a failed deletion is logged, the backup stands). Under a local root every
folder is opened without following links (`openat`, `O_NOFOLLOW`), files are written aside
(`0600`, a new file) and renamed. A **restore** downloads a file and reads it whole first (SQL decompressed to a private file, a
MongoDB archive checked): a corrupt file changes nothing. It then feeds it to
`psql -v ON_ERROR_STOP=1 --single-transaction`, `mysql` or `mariadb` (`mongorestore --drop`, the
archive's database renamed to the one asked), in place. A command whose caller goes away (a job
timeout) is killed in the container (its processes carry a mark), never left half fed.

Schedules: every minute the `database-backups` tick (RustCMS W4) queues one run per enabled
configuration whose cron fired since its last slot (missed slots are not caught up), the slot
claimed in the database.

### Actions

The Actions page (`/admin/p/actions`, like EasyPanel's) lists the last 100 deployments, backup runs
and Docker actions (pulls, prunes, container actions), newest first, each kind with its permissions
(deployments: `deployments.view_any` and `deployments.view`)
(Docker actions: the caller's own, every one with `system.manage`); a row opens its service's tab or
its log.

### Notifications

As EasyPanel's (Settings → Notifications). A channel is Discord or Slack (their webhook URL), a
Telegram bot (token and chat), email (SMTP: port 465 TLS, else STARTTLS required) or a webhook (a
JSON `{event, ok, title, details, data}` POST). Its URL, token or SMTP password is sealed (context
`notification_channels.secret`); changing a channel's kind or SMTP server needs it entered again
(never the old one sent elsewhere). Each channel picks its events: deployments (Deploy and Rollback
that succeeded, and every failure), database backups and restores, and the server's disk at or over
its threshold (checked every 5 minutes, said at most every 6 hours per channel). Sent in the
background, 15 s per channel, never retried; a failure is logged (never the URL). "Send test
notification" sends at once and answers why it failed.

### Secrets

A service's environment variables (`services.env`) and registry password
(`services.registry_password`) are RustCMS encrypted columns (`Encrypted<T>`, sealed with `APP_KEY`):
the resource form shows the variables' names and whether a password is set, never a value; the
Environment tab shows the values to users with `services.secrets` (an audited read); the audit log
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
another's directly (domains and published ports remain public). Once a project is deleted, its
network goes within 5 minutes (the `orphan-networks` schedule): Traefik detached, then the network
removed (one still in use is left for the next time).

## The apps interface

`Dashboard` (`/admin/p/dashboard`, React, RustCMS extension host v2) is the EasyPanel-like
interface: server tiles and projects with their service cards; a project; a service with its header
(status, Deploy, Start/Stop, Restart, Open) and tabs (Overview, Deployments, Environment, Domains,
Source, Storage, Advanced), by sub-path `<project>/<service>/<tab>`. It reads
`/admin/api/x/apps/...` (`apps_controller.rs`) and writes through the resources' API and actions, so
the resources' policies, validation, audit and encrypted fields apply unchanged. The environment is
edited as `.env` text (`services/dotenv.rs`): `PUT /apps/services/{id}/env` seals it; reading it in
clear needs `services.secrets` and is audited.

## Data

MySQL 8.4 (`docker-compose.yml` for development). Besides the framework's tables (users, roles,
sessions, audit log, jobs, schedule slots, …), the panel adds `actions`, then for Phase 2 `projects`,
`services`, `service_domains`, `service_ports`, `service_mounts` and `deployments` (each with its
configuration snapshot, pinned image, status and log), then `storage_providers`,
`database_backups`, `backup_runs` and `notification_channels`.
