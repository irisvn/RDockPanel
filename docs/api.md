# The panel's routes

All routes are served under `/admin/api/x` on the admin session (the panel's cookie). They are meant
for the panel's own pages, not for scripts: token-authenticated API routes come later (Phase 3).

- Writes (`POST`, `DELETE`) need an allowed `Origin` and the session's CSRF token in `X-CSRF-Token`
  (`GET /admin/api/auth/csrf` returns it).
- Each route requires the permission shown; `{id}` is a container id or name. Errors from the panel
  are JSON with a stable `code`; a malformed path or query parameter (e.g. `?force=maybe`) gets a
  plain-text 400.

## Docker (`/docker`)

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/info` | docker.view | engine version, OS, CPUs, memory, counts, swarm role |
| GET | `/containers` | docker.view | the containers the caller may see |
| GET | `/containers/{id}` | docker.view | inspect (secrets redacted) |
| POST | `/containers/{id}/{action}` | docker.manage | `start`, `stop`, `restart`, `kill`, `pause`, `unpause` |
| DELETE | `/containers/{id}?force=` | docker.manage | remove |
| GET | `/containers/{id}/logs?tail=&since=&timestamps=` | docker.logs | the last lines: `tail` up to 5,000 (default 500); reading stops after about 2 MiB of text; lines are cut to 16 KiB |
| GET (SSE) | `/containers/{id}/logs/follow?tail=&since=&timestamps=` | docker.logs | `lines` events: the tail, then new lines |
| GET (SSE) | `/containers/{id}/stats` | docker.view | `stats` events about every second |
| GET (WS) | `/containers/{id}/exec?cols=&rows=` | docker.exec | a terminal: binary messages both ways; text `{"type":"resize","cols":80,"rows":24}`; the server sends `{"type":"exit"}` or `{"type":"idle"}` before closing |
| GET | `/images` | docker.view | images with tags, size, use |
| POST | `/images/pull` | docker.manage | `{"image": "nginx:1.27"}` → 202 with the queued action |
| DELETE | `/images/{tag or id}?force=` | docker.manage | remove (force needs system.manage) |
| POST | `/prune` | docker.manage | stopped containers and dangling images, never the panel's own |
| GET | `/actions/{id}` | docker.view | one action (own actions; all with system.manage) |
| GET / POST | `/volumes` | docker.view / docker.manage | list / create `{"name"}` |
| DELETE | `/volumes/{name}` | docker.manage | remove |
| GET / POST | `/networks` | docker.view / docker.manage | list / create `{"name","driver":"bridge"\|"overlay"}` |
| DELETE | `/networks/{id or name}` | docker.manage | remove |

## Apps (`/apps`)

What the apps interface reads; it writes through the resources' API
(`/admin/api/resources/{projects,services,service_domains,service_ports,service_mounts}/records`, and
their actions), so the resources' policies, validation and audit apply.

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/overview` | projects.view_any | every project with its services (status, kind, primary domain) |
| GET | `/services/{id}` | services.view | a service's page: configuration (names of its environment variables, never values), domains, ports, mounts, pending changes, recent deployments (with deployments.view) |
| GET | `/deployments/{id}` | deployments.view | one deployment with its log |
| GET | `/services/{id}/stats` | services.view | the Overview's monitor: CPU and memory of each running task on this node, and their sum (two Docker stats samples, at most 3 s) |
| POST | `/projects/{id}/services` | services.create | `{"name","kind","image"?,"database"?:{database,user,password,root_password}}`: an app (`kind` `app`, `image` required) or a database (`postgres`, `mysql`, `mariadb`, `mongo`, `redis`: credentials generated when left out); 422 on bad names, images, taken names; audited (`services.create`) |
| POST | `/services/{id}/deploy-key` | services.update | a new SSH deploy key (ed25519): `{"public_key"}`, the private half sealed; audited |
| POST | `/services/{id}/trigger` | services.deploy | a new deployment trigger URL, shown once: `{"url"}`; the old one stops working; audited |
| POST | `/services/{id}/basic-auth` | services.update | `{"username","password"}`: a user of its basic authentication (bcrypt; the same user replaced); audited |
| DELETE | `/services/{id}/basic-auth/{user}` | services.update | removes one; audited |
| GET | `/services/{id}/credentials` | services.secrets | a database's passwords and internal URL; audited first |
| GET | `/services/{id}/env` | services.secrets | `{"text","revision"}`: the environment as `.env` text, in clear; audited first (`services.env.read`, names only: no audit, no values) |
| PUT | `/services/{id}/env` | services.update | `{"text","revision"?}` replaces the environment (sealed); with the revision read, 409 `env_stale` when it changed meanwhile; 422 names the bad line; audited (`services.env.update`, names only) |

A database's page includes `database`: engine, internal host, port, user, database; never its
passwords. `GET /services/{id}/credentials` (services.secrets) answers `{"password",
"root_password","url"}`, audited first (`services.credentials.read`).

`/overview` and `/services/{id}` include each kind of record only with its resource's permission
(`services.view_any`, `service_domains.view_any`, `service_ports.view_any`, `service_mounts.view_any`).

## Templates (`/apps`)

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/templates` | projects.view_any | the catalog: slug, name, description, tags, logo file |
| POST | `/templates/refresh` | services.create | fetch the catalog and templates again |
| GET | `/templates/{slug}` | projects.view_any | name, description, instructions, links, `schema` (the form) |
| GET | `/templates/{slug}/logo` | projects.view_any | the logo (image, `sandbox` CSP) |
| POST | `/projects/{id}/templates/{slug}` | services.create (+ projects.view; services.deploy to deploy) | `{"input":{…},"deploy":bool}`: runs the generator, creates its services (databases first), queues their deployments when asked; `{"project","services","warnings"}` |
| POST | `/projects/{id}/schema` | same | `{"schema":{"services":[…]},"deploy":bool}`: "Create from Schema" |

## Backups (`/apps`)

Configurations and providers are written through the `database_backups` and `storage_providers`
resources.

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/services/{id}/backups` | database_backups.view_any | a database's Backups tab: `supported`, `configs`, the last 30 `runs`, `providers` (with storage_providers.view_any) |
| POST | `/backups/{id}/run` | backups.run | runs a configuration now: 202 with the run; audited |
| GET | `/backup-runs/{id}` | database_backups.view_any | a run with its log |
| POST | `/services/{id}/restore` | backups.restore | `{"storage_provider_id","file","database"}`: restores a file in place: 202 with the run; 422 on a bad file or database; audited |
| GET | `/storage-providers` | storage_providers.view_any | the providers (never the secret key: `secret_set`) |
| GET | `/storage-providers/{id}/files?path=` | backups.restore | the files of a folder, newest first |
| POST | `/storage-providers/{id}/check` | storage_providers.update | writes, lists and deletes a small file: `{"ok","error"?}` |

## Actions (`/apps`)

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/actions` | projects.view_any | the last 100 deployments (with deployments.view_any and deployments.view), backup runs (database_backups.view_any) and Docker actions (docker.view; own unless system.manage): `{type, id, what, status, error, detail?, target?: {project, service}, created_at, finished_at}` |

## Notifications (`/apps`)

Channels are written through the `notification_channels` resource.

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/notification-channels` | notification_channels.view_any | the channels (never their secret: `secret_set`) |
| POST | `/notification-channels/{id}/test` | notification_channels.update | sends a test notification now: `{"ok","error"?}` |

## Deployments (`/deploy`)

| Method | Path | Permission | |
|---|---|---|---|
| GET (SSE) | `/deployments/{id}/log` | deployments.view | `log` events (`{"append": "…"}`, or `{"reset": "…"}` with the whole log), `status` events (the deployment without its log) about every second; ends once the deployment finished |

Deploy, Start, Stop, Restart, Destroy, Republish routes (`republish`) and Rollback are resource
actions (`POST /admin/api/resources/services/actions/{name}`, `…/deployments/actions/rollback`). The actions
that run a deployment (all but Republish) answer 409 `deployment_in_progress` while another one of the
service is under way.

## Deployment trigger (public)

`POST /api/v1/deploy/{id}?token={token}` (no session, no CSRF: the token is the secret) queues a Deploy of
the service: 202 `{"deployment"}`; 404 for an unknown service or a wrong token; 409 while another
deployment of the service is under way. Audited as `services.deploy.trigger` (system actor).

## Maintenance page (public)

`GET /api/v1/maintenance/{project}_{service}`: the page of a service in maintenance (503, HTML);
404 otherwise. Traefik routes the service's domains there while it is in maintenance.

## GitHub webhook (public)

`POST /api/v1/github/{id}` (no session: `X-Hub-Signature-256` is the proof): a `push` to the
service's branch queues a Deploy (202 `{"deployment"}`); `ping` answers 200; other events or
branches 200 `{"ignored"}`; a bad signature 404. Made by `POST /apps/services/{id}/github-webhook`
(services.deploy): `{"url","secret"}`, shown once, audited.

## Server (`/server`)

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/info` | server.view | hostname, OS, kernel, architecture, cores, uptime |
| GET | `/samples` | server.view | the samples kept (5 minutes) |
| GET (SSE) | `/live` | server.view | `history`, then a `sample` every 2 s; `resync` when samples were missed |

## Streams and sockets

Event streams cut short end with `event: closed` (`signed_out`, `forbidden`, `shutdown`, `lagging`,
`error`); a stream that is over (a finished deployment's log) just ends.
WebSockets close with 1000 (the session ended), 4401 (signed out), 4403 (forbidden), 1001 (shutdown),
4008 (lagging), 1009 (message too big) or 1011 (error). Pages use the panel's
`ctx.realtime.events(...)`, which reconnects with backoff, and `ctx.realtime.socket(...)`, which the
terminal opens without reconnect (a new connection would be a new shell: "New session"). Both close
on unmount and session changes. See the RustCMS guide, *Realtime events*.
