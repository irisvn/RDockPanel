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
| GET | `/services/{id}/env` | services.secrets | `{"text","revision"}`: the environment as `.env` text, in clear; audited first (`services.env.read`, names only: no audit, no values) |
| PUT | `/services/{id}/env` | services.update | `{"text","revision"?}` replaces the environment (sealed); with the revision read, 409 `env_stale` when it changed meanwhile; 422 names the bad line; audited (`services.env.update`, names only) |

`/overview` and `/services/{id}` include each kind of record only with its resource's permission
(`services.view_any`, `service_domains.view_any`, `service_ports.view_any`, `service_mounts.view_any`).

## Deployments (`/deploy`)

| Method | Path | Permission | |
|---|---|---|---|
| GET (SSE) | `/deployments/{id}/log` | deployments.view | `log` events (`{"append": "…"}`, or `{"reset": "…"}` with the whole log), `status` events (the deployment without its log) about every second; ends once the deployment finished |

Deploy, Start, Stop, Restart, Destroy, Republish routes (`republish`) and Rollback are resource
actions (`POST /admin/api/resources/services/actions/{name}`, `…/deployments/actions/rollback`). The actions
that run a deployment (all but Republish) answer 409 `deployment_in_progress` while another one of the
service is under way.

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
