# Architecture

RDockPanel is a [RustCMS](https://github.com/irisvn/RustCMS-Core) application: one binary
(`rdockpanel`) that serves the admin panel, its JSON/event/WebSocket routes, and runs queue
workers. It follows the standard RustCMS layout (`rustcms make:*` generators). The main pieces:

```
src/
  bin/rdp.rs        the `rdp` command line (the public API)
  bootstrap/        the app from its providers, then its API and admin routes, the homepage at /
  providers/        auth (permissions), admin panel (pages, scripts, styles), app (services, jobs, boot)
  routes/admin.rs   the panel's own routes under /admin/api/x (AdminRoutes)
  routes/api.rs     /api/v1: the public API (tokens), deployment triggers, GitHub webhooks,
                    maintenance pages
  http/controllers/ docker (containers, images, volumes, networks, terminals, a service's logs),
                    server, deploy (a deployment's live log), apps, domains, templates, backups,
                    actions, notifications, traefik, system, members, tokens, git, tunnel,
                    branding, home; api_v1 and trigger (/api/v1)
  services/         docker_service (+ bollard), system_stats_service, live_service, action_service;
                    deploy_rules_service (rules, [panel] config), deploy_service (spec and routes),
                    deployment_service (running deployments), traefik_service (route files),
                    build_service, compose_service, template_service, backup_service, …
  models/           action (the actions log); project, service, service_*, deployment, … (see Data)
  jobs/             pull_image, run_deployment, publish_routes, run_backup
  policies/         who may do what: permissions.rs (the panel's own permission names), one
                    policy per resource (project, service, domain, deployment, …), ChildPolicy
                    for a service's records, and the helpers asking a user's roles and project
                    role (may, admits, members_only, write)
  admin/pages/      Dashboard, Domains, Firewall, Actions, Settings, Containers, Images, Volumes, Networks (React)
  admin/resources/  Projects, Services, Domains, Ports, Mounts, Redirects, Scripts, Deployments,
                    backups, storage providers, notification channels, certificate resolvers
                    (fields, forms, tables, scopes, hooks; their policies in policies/)
  admin/slots/      the service page's aside (state, actions, live deployment log)
resources/admin/    src/: every page in React (built to js/rdockpanel/dashboard.js), css/rdockpanel/
                    (dashboard.css, xterm.css), js/rdockpanel/vendor/ (xterm.js)
resources/home/     the public homepage (HTML, CSS, its screens' viewer, images)
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

1. The browser loads `/admin` (the RustCMS SPA). The panel's bundle registers its React pages
   (`registerPage("rdockpanel/containers", …)`).
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
pulls), `deploy` (deployments, route publishing) and `backup` (backups and restores, up to 3 hours),
configured in `config/queue.toml`. Jobs: `pull_image`, `run_deployment`, `publish_routes`,
`run_backup`.

## Projects and services (Phase 2)

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

### Templates

`services/template_service.rs`, `services/template_engine.rs`, `templates_controller.rs`. The
catalog (`list.json`), each template's `meta.yaml` and `index.ts`, and logos are fetched from
`[panel] templates_url` (the `easypanel-io/templates` repository on GitHub, raw files) when the
Templates dialog opens, and
kept six hours; nothing is bundled (the repository has no license). Logos go through the panel
(`/apps/templates/{slug}/logo`): its CSP loads images from itself only.

A template's form is its JSON Schema (defaults filled in); its
TypeScript generator (at most 256 KB), types stripped by oxc, runs in QuickJS with a 64 MB, 3 s
budget, in a child process of the panel's own binary (`rdockpanel __rdockpanel-run-template`: CPU
and, on Linux, memory limits, killed after 15 s; a crash takes the child down, never the panel),
and nothing to import but `~templates-utils` (`randomString`, `randomPassword`, `bcryptHash`), its `./meta` and
`crypto` (`randomBytes`, `randomUUID`; also `require("crypto")` and `Buffer.from` for older
templates); no file system, no network. Its `{services: [{type, data}]}` output becomes
services through `creation_service` (one transaction: all or none): apps from an image (env as
`.env` text, quoted and multi-line values, placeholders resolved at deploy, `$(EASYPANEL_DOMAIN)` domains as generated domains,
volumes, files, ports, command, replicas, zero downtime) and databases (their options), databases
first. What cannot be done is said, never dropped silently: host directories, redirects, resources
and registry credentials are left out with a warning; what a service needs to be safe or to run
(basic authentication, Linux capabilities, sysctls) is created but refused for "Deploy now"; Git, GitHub and
Dockerfile sources become builds (with a Dockerfile; a template building any other way is reported
as not supported), and
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

- **Placeholders** (`services/placeholders.rs`), as templates write them, are replaced
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
(a commit: the whole history, then that commit), a path in the repository and the repository's
Dockerfile (`dockerfile_path`); `builder` takes `dockerfile` only (services that had another
builder were moved to it by the migration `builds_use_a_dockerfile`). A `dockerfile` source is a
Dockerfile written in the panel. A
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

### Wildcard domains and middlewares

A domain's host may be a wildcard, `*.example.com`: every name one level under it, as Traefik's
`HostRegexp`, made by the panel's administrators only (`system.manage`). Routers carry
priorities: an exact host always wins over a wildcard. Its HTTPS certificate is asked for by name
(`tls.domains`) and needs a DNS-challenge resolver (the installer's uses the HTTP
challenge: Traefik serves its default certificate then). A domain may name middlewares of its own,
defined in Traefik's files (`ratelimit@file`, in a file of `/etc/rdockpanel/traefik/dynamic` the
panel does not write), applied after the panel's (redirects, authentication).

### Redirects and basic authentication

The Redirects and Security tabs. A service's redirects (`service_redirects`: a regular
expression over the request's URL, its replacement, permanent or not, on or off) and the users of
its HTTP basic authentication (`service_basic_auth`: bcrypt hashes, never shown) become Traefik
middlewares (`redirectRegex`, then `basicAuth`) on every router of its domains. They apply when
saved (the routes written again, no deployment): a user added, removed or given a new password signs
in so at once. A deployment's snapshot records them (users as a digest of their hash, never the hash).
Regular expressions are checked as Go's (inline flags `i`, `m`, `s`, `U` only), and no value
written to Traefik's files may contain `{{` or `}}` (Traefik reads them as Go templates). Templates' `basicAuth` and `redirects` are created with the service.

### Rate limit

The Security tab's Rate limit (`services.rate_limit_*`: an average per second, minute or hour, a
burst, exempt addresses) becomes Traefik's `rateLimit` on the routers serving each domain, before the
redirects and the authentication; the redirects to HTTPS are not limited. It applies when saved, like
basic authentication. Who the visitor is depends on the peer, so each serving router gets variants:

| Router | Matches | Counted by |
|---|---|---|
| `<name>` | anyone else | the peer (`ipStrategy.ipv6Subnet: 64`: an IPv6 visitor by their /64) |
| `<name>-exempt` | an exempt peer (`ClientIP`) | not limited |
| `<name>-cf` | a Cloudflare edge (`ClientIP` of its ranges) | the visitor it names (`depth: 1`, the last `X-Forwarded-For` entry) |
| `<name>-tunnel`, `-tunnel-secure` | cloudflared, on the `tunnel` entrypoint | the visitor it names |

Traefik keeps `X-Forwarded-For` only from peers it trusts (`forwardedHeaders.trustedIPs`):
Cloudflare's ranges on `web` and `websecure` (the installer's arguments, and the panel's at boot), the
tunnel's network on `tunnel` (set when the tunnel is turned on: Traefik is on every project's network,
so a container could reach that entrypoint, and its header is dropped). Anyone else's is dropped, so
nobody picks their own key, and the exempt addresses match the peer only (never a Cloudflare edge,
whose ranges they may not overlap). Every service router's priority is its rule's (exact hosts first,
then length) times 4, the variants +1 and +2: never above another route. Counters are Traefik's, in
memory: per router (so per domain), reset when Traefik restarts or reloads.

### Capacity check

`services/capacity_service.rs` tells whether the server has room for what is about to run. It is
advisory: the panel asks before (`POST /apps/capacity`, a template's dry run), and every deployment's
run checks again and logs it; nothing stops a deployment but the user.

What a service takes, by task, in this order: what its own running tasks use (the same image) (the largest of up to
three, one Docker stats sample each), or the same image in its project when the caller sees its services (`observed`); its memory
limit (`limit`); else a typical figure for its kind (mysql 450 MB, mongo 300, mariadb 200,
postgres 128, redis 32, app 192: `default`). A Compose file counts its services (an app's each);
one from a repository is `unknown`. Then what the action changes:

| | Memory | Disk |
|---|---|---|
| more tasks than run now (a start: all), or another image | what they take more than the running ones use | |
| a rolling update, start-first | a new task beside the one it replaces, while it rolls (parallelism 1; reservations too) | |
| a build (Deploy, Force rebuild, or never built yet) | 1 GB while it builds | 3 GB |
| an image pulled (not here yet, or a Force rebuild) | | 1 GB |

Against the host (the panel's samples: `MemAvailable`, swap, the disk of `/`, the CPU's average):

| Level | When |
|---|---|
| `impossible` | Swarm's memory reservations (its tasks placed on a node) plus the new ones exceed the node's memory: the task would stay pending. The UI refuses to deploy it |
| `short` | the memory needed exceeds what is available, or less than 1 GB of disk would stay free. The UI asks for "deploy anyway" |
| `tight` | less than 15% of the memory (at least 256 MB) stays free, swap over half used, less than 2 GB of disk, or the CPU over 90%. The UI says it |
| `ok` | otherwise, or nothing more runs |
| `unknown` | the host's figures are not read yet: nothing said |

The server's figures go to who sees the server (`server.view`); everyone else sees the level, the
need and why it does not fit, never the server's state, and `observed` reads only their project's
services. A deployment's log says the level and the need (no host figures). A service whose
deployment runs short (whoever asked: the UI, the API, a webhook) is notified once to the channels
hearing `service` events, again only after it had room (kept in memory: a restart may say it
again). A running Compose service counts nothing more (what it adds is not compared).

### Compose services

A service of kind `compose` (made by the panel's administrators only:
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

A **storage provider** (Settings) is a local directory (absolute; in the
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

The Actions page (`/admin/p/actions`) lists the last 100 deployments, backup runs
and Docker actions (pulls, prunes, container actions), newest first, each kind with its permissions
(deployments: `deployments.view_any` and `deployments.view`)
(Docker actions: the caller's own, every one with `system.manage`); a project member sees their
projects' only; a row opens its service's tab or
its log.

### Notifications

Settings → Notifications. A channel is Discord or Slack (their webhook URL), a
Telegram bot (token and chat), email (SMTP: port 465 TLS, else STARTTLS required) or a webhook (a
JSON `{event, ok, title, details, data}` POST). Its URL, token or SMTP password is sealed (context
`notification_channels.secret`); changing a channel's kind or SMTP server needs it entered again
(never the old one sent elsewhere). Each channel picks its events: deployments (Deploy and Rollback
that succeeded, and every failure), database backups and restores, and the server's disk at or over
its threshold (checked every 5 minutes, said at most every 6 hours per channel). Sent in the
background, 15 s per channel, never retried; a failure is logged (never the URL). "Send test
notification" sends at once and answers why it failed.

### Capabilities and sysctls

The Advanced tab. Tini is every container's init. A service's added and dropped
Linux capabilities (`CAP_` prefix optional) and sysctls (`key=value` lines) become its container
spec's `CapabilityAdd`, `CapabilityDrop` and `Sysctls`. Adding capabilities, sysctls or supplemental groups reaches the
host's kernel: it needs `system.manage` (in the panel and for templates); dropping does not.

### Scripts

The Scripts tab. A service's scripts (`service_scripts`: a name, a shell command) run with
`sh -c` in one of its running containers (`POST /apps/scripts/{id}/run`, `docker.exec` as a
terminal), at most 25 seconds (then killed in the container; within the request's own timeout), only the command its runner saw (409 otherwise), only in a service the panel deployed; run apart from the request (its action always finished); a run left running (a stop) is killed by the every-minute reaper after 5 minutes, then finished (`script.run` actions hold its mark; a failed kill is tried again), their output's last 64 KB returned;
each run is audited (its name, never its output). Not for Compose services.

### Maintenance mode

The Maintenance tab. A service in maintenance (applied when saved) has its
domains routed to the panel (`[panel] internal_url`, `http://rdockpanel:3000` installed; empty: no
maintenance mode) at `/api/v1/maintenance/<project>_<service>`, answered 503 with `Retry-After`
(until the time it is back: a minute to a day; five minutes when none), while it is in maintenance
(turned off: its domains lead to the service again at once). The page
(`services/maintenance_service.rs`) is:

- **Default:** the panel's shared page (Settings → Maintenance page) when there is one, else the
  built-in page: the logo, the title, the subtitle, when it is back (in the offset it was written in)
  and the contact (an email as text, an https link as a link), in the visitor's language (English or
  Vietnamese, `Accept-Language`), light or dark.
- **Its own HTML** (`maintenance_html`, up to 256 KB).

HTML pages fill in `{{title}}`, `{{subtitle}}`, `{{eta}}`, `{{eta_iso}}`, `{{contact}}`,
`{{contact_href}}`, `{{logo}}` (a data URL), `{{service}}` and `{{project}}` (escaped; others stay as
written). A page larger than 1 MB once filled in, or using the logo more than 4 times, is replaced by
the built-in page (which leaves a logo over 128 KB out). The logo is the service's own
(`service_logos`: PNG, JPEG or WebP up to 256 KB) or the panel's (Branding: light and dark).

Every page runs in a sandbox (`Content-Security-Policy: sandbox; default-src 'none'; style-src
'unsafe-inline' https:; img-src data: https:; …`): no script, no form, an opaque origin. It is
served only for the service's own domains (the request's `Host`, which Traefik keeps; a wildcard's
single label): through the panel's domain or an address it is 404. The tab's preview is rendered
by the panel from what is being written (kept five minutes, its author's only, at most 3 each and 20
in all) and shown in a sandboxed frame with the same policy (`frame-ancestors 'self'`).

**Addresses let through** (`maintenance_bypass`, checked like rate limit exemptions) still reach the
service: each router showing the page has a twin routing as it does out of maintenance (its
redirects and authentication, without the rate limit), just above it: `-bypass` (+1) for exempt
peers (`ClientIP`), `-bypass-cf` (+2) for a Cloudflare edge naming an exact address last in
`X-Forwarded-For` (`HeaderRegexp`), and on the tunnel's entrypoint `-bypass` (+2, above its scheme's
routers). Networks match direct visitors only. Its notes (Overview) are for the team only and never
deployed.

### Auto deploy from GitHub

A Git service may have a GitHub webhook (`/api/v1/github/{id}`): its secret (40 characters, shown
once, sealed: context `services.webhook_secret`) signs each delivery (`X-Hub-Signature-256`,
HMAC-SHA256, compared in constant time). A `push` to the service's branch (the repository's default
branch when none is set; a full `refs/…` name as given) queues a Deploy; one arriving while a
deployment runs is kept (pushes coalesce) and deployed once it ends; a `ping` answers; other events
and branches are ignored. A missing or wrong
signature answers 404, as an unknown service.

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

RustCMS seals 32 KiB at most (the value's JSON). A service's environment is one sealed value, and
so are its files together (a deployment keeps them so): the Environment tab, the forms, templates and
Deploy refuse more with a field error (or a 400 naming it), never a failure on the way. Values kept in
clear that may be longer than MySQL's 64 KiB `TEXT` (a Compose file, notes, the shared maintenance
page and the other settings, a certificate chain) are `MEDIUMTEXT` there.

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

The service page's aside (`ServiceLiveSlot`, `docker/ServiceLive.tsx`) shows the state, pending changes, the
action buttons and the latest deployment, its log followed live over SSE.

### Routes (Traefik)

`services/traefik_service.rs`. Traefik reads one file per service, `<project>_<service>.yml`, from
`[panel] traefik_dir` (its file provider; `storage/traefik/dynamic` in development). The panel
writes it whole (a hidden temporary file, then renamed) after each deployment and whenever a route's
own setting is saved: the containers of the deployment that made the service what it is (its
configuration hash; a failed deployment leaves what still runs), with its domains, redirects, basic
authentication, rate limit and maintenance **as saved**. Saving one of those queues a `deploy.publish` job (from
the resources' `after_save` and the basic authentication routes, in the same transaction): it is
applied without a deployment, the containers untouched, and does not count as a pending change (only
what is in the Swarm service does: image, environment, ports, mounts, resources…). A Compose
service's domains are the exception: they name its file's services, checked as it deploys, so they
still change by deploying. Stop and Destroy
remove the file before the tasks go (if that fails, the deployment goes on with a warning in its log).
Whether the write worked is the deployment's `published`; the service's **Republish routes** action
(the same job) writes it again. At boot every file is written again from the database, and
files of services that no longer exist are removed.

Traefik reaches a service as `http://<project>_<service>:<port>` on the project's network: before
the first deployment with a domain (or the routes' job, for a domain added since), the panel attaches `[panel] traefik_service` (`rdp-traefik`) to
`rdp_<project>` (Swarm restarts Traefik then: a few seconds without routing); when it cannot, the
deployment fails rather than publish domains that lead nowhere. Writes are serialized (one lock in
the panel's process, workers included), each reading the database at that moment, so the last one
written is always the latest state. Projects share no network, so a project's services cannot call
another's directly (domains and published ports remain public). Once a project is deleted, its
network goes within 5 minutes (the `orphan-networks` schedule): Traefik detached, then the network
removed (one still in use is left for the next time). Only networks this panel made are removed:
they carry its instance id (`rdockpanel.instance`, derived from its `APP_KEY`), so another panel's
or a test's networks on the same Swarm are left alone.

**Certificates.** A domain's HTTPS certificate comes from its certificate resolver, else the panel's
default (`[panel] cert_resolver`: the installer's `letsencrypt`, HTTP challenge). The panel's own
resolvers (`cert_resolvers`, `cert_service.rs`) are Let's Encrypt over HTTP or over DNS with a
provider's token, the only way to a wildcard's certificate. Traefik reads its static configuration
from its arguments only, so "Apply" rewrites the arguments the panel owns
(`--certificatesresolvers.<name>.…`, never the installer's `letsencrypt`), gives each DNS token as
a Swarm secret read through `<VAR>_FILE`, labels Traefik with the digest of what it applied, and
keeps everything else (networks, mounts, ports). An installer run replaces the arguments: every two
minutes the panel gives them again when the label says they were applied but they are missing
(`traefik-resolvers` schedule); a change not applied yet waits for "Apply". One host has one
certificate (Traefik picks it by name before any path), so every HTTPS domain of a host chooses the
same resolver; each resolver keeps its own ACME storage (`/etc/traefik/acme/<name>.json`). A
resolver in use (by a domain, or a deployed route) cannot be deleted. The panel's own host
(`APP_URL`) is never a service's domain, and its route outranks theirs.

Uploaded certificates (`certificates`, `certificate_service.rs`) are read before they are kept: the
key must be the certificate's, each certificate of the chain signed by the next, valid now, for
servers, naming DNS names; the key is sealed. The panel writes them for Traefik under
`<traefik_dir>/certs/` (files `0600`, directory `0700`, names of its own) and lists them in
`rdp-certificates.yml` with Traefik's own paths (`[panel] traefik_mount`); Traefik serves the one
covering a name by SNI, with no restart. A domain choosing one gets a router without a resolver; it
must cover the host (a wildcard host only by a wildcard certificate). A renewed one replaces it in
place (domains refer to its id: nothing to deploy). The panel rewrites the files at boot, and warns
the notification channels 14, 7 and 1 days before one expires, and once expired (each once).

Settings → Traefik shows Traefik's state (image, published ports, tasks, last update: never its
arguments), restarts it (its tasks replaced, through the same lock as its other changes), streams its
logs, and keeps the administrators' middlewares (`panel_settings`, written to
`rdp-custom-middlewares.yml`, no restart). Those are named `custom-<name>` and limited to the types
that change only the request or the answer (headers, rate and in-flight limits, IP allow lists,
redirects, prefixes, compression, retries, inline basic or digest users, chains of their own): none
that sends requests elsewhere (`forwardAuth`, `errors`, plugins), reads Traefik's files
(`usersFile`), or is a Go template; routers stay the panel's. A domain uses one as
`custom-<name>@file`.

Settings → Panel shows the panel's domain, Let's Encrypt email and apps' domain as the installer set
them, checks new ones (host names, an email, a name a service already uses, DNS records: warned of,
never refused, as proxies and NAT answer otherwise) and gives the installer command applying them.
The panel never moves itself: its own service, Traefik's `letsencrypt` resolver and its route would
change at once, with no way back if it did not come up. The installer reads `install.conf` as data.

## The apps interface

`Dashboard` (`/admin/p/dashboard`, React, RustCMS extension host v2) is the apps interface: server
tiles and projects with their service cards; a project; a service with its header (status, Deploy,
Start/Stop, Restart, Open, Logs, Shell) and tabs (Overview, Credentials (databases), Backups
(databases and services with volumes), Deployments, Environment, Domains, Redirects, Security,
Maintenance, Scripts, Source, Storage, Advanced; a database has no Redirects, Security, Maintenance
or Scripts, a Compose service no Scripts), by sub-path `<project>/<service>/<tab>`. It reads
`/admin/api/x/apps/...` (`apps_controller.rs`) and writes through the resources' API and actions, so
the resources' policies, validation, audit and encrypted fields apply unchanged. The environment is
edited as `.env` text (`services/dotenv.rs`): `PUT /apps/services/{id}/env` seals it; reading it in
clear needs `services.secrets` and is audited.

The sidebar: Dashboard (the panel's home, `AdminProvider::home`, RustCMS 0.5.1), Domains, Firewall
(`system.manage`, [firewall.md](firewall.md)), Actions, Settings (its tabs: Panel, Version, Traefik, Certificates, Cloudflare Tunnel, Branding, Maintenance page, Git,
Backups, Notifications, API tokens, each by its permission), then the Docker pages, then RustCMS's System. The resources (projects, services,
domains, …) have no sidebar entry (`navigation(false)`): their routes and API stay, as the React pages
write through them, and an administrator can still open `/admin/r/<slug>`. The panel is light by default,
whatever the operating system prefers (`Theme::color_scheme`); each user can pick Light, Dark or System
at the bottom of the sidebar (RustCMS 0.6.1, kept in the browser).

## Data

MySQL 8.4 (`docker-compose.yml` for development). Besides the framework's tables (users, roles,
sessions, audit log, API tokens, jobs, schedule slots, …), the panel adds `actions`, then for Phase 2
`projects`, `services`, `service_domains`, `service_ports`, `service_mounts` and `deployments` (each
with its configuration snapshot, pinned image, status and log), then `service_redirects`,
`service_basic_auth`, `service_scripts`, `storage_providers`, `database_backups`, `backup_runs`,
`notification_channels`, `cert_resolvers`, `certificates`, `panel_settings` (custom middlewares,
the homepage switch, …), `project_members`, `git_providers` and `branding_images`.
