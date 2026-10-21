# The panel's routes

Scripts and CI use the **public API** (`/api/v1`, with a token; below). The other routes are served
under `/admin/api/x` on the admin session (the panel's cookie), for the panel's own pages.

## Public API (`/api/v1`)

Make a token in **Settings → API tokens** (each user their own): a name, scopes, an expiry (at most
365 days). It is shown once. Send it as `Authorization: Bearer rcms_…`. A token does what its user
may — their roles, and their roles in projects — and **never beyond its scopes**: both must allow.
Revoke it there.

| Scope | Allows |
|---|---|
| `read` | projects, services, deployments and domains: seen |
| `deploy` | deploy and restart services |
| `write` | change and add services and domains |
| `secrets` | read environment values (audited) |
| `logs` | read services' logs |

A service is named by its id or by `project/service` (URL-encoded: `shop%2Fweb`). Errors are JSON
`{"code", "message"}`: 401 without a valid token, 403 outside its scopes or its user's projects,
404 for what does not exist or is in a project the user is no member of (the same answer: no
telling which). A token holding anything beyond the scopes (RustCMS's console `*`) is refused here.
Requests are rate-limited per user and route (120 a minute; deploy and restart 10): 429 beyond.
Deploying a Compose service or a privileged one (added capabilities, sysctls or groups) through the API is
refused: those need `system.manage`, which no token carries (deploy them in the panel).
The `write` and `logs` scopes are for routes to come.

| Method | Path | Scope | |
|---|---|---|---|
| GET | `/me` | any | the token's user (id) and scopes |
| GET | `/projects` | read | the projects the user sees, with their services |
| GET | `/services/{id}` | read | the service: image, status, replicas, domains, latest deployment (each when the token may see them) |
| POST | `/services/{id}/deploy` | deploy | queues a deployment: 202 `{"deployment": …}` |
| POST | `/services/{id}/restart` | deploy | queues a restart: 202 |
| GET | `/deployments/{id}` | read | its status, error and log |
| GET | `/services/{id}/env` | secrets | `{"env": {NAME: value}}` (audited, as the panel's own read) |
| GET | `/services/{id}/domains` | read | its domains |

### The `rdp` command line

`rdp` (built with the panel: `cargo build --release --bin rdp`) uses this API:

```sh
rdp login https://panel.example.com     # asks for the token (stdin), checks it, keeps it (mode 0600)
rdp projects
rdp status shop/web
rdp deploy shop/web --wait              # waits for the deployment, prints its log
rdp restart shop/web
rdp logs 42                             # a deployment's log (waits until it ends)
rdp env shop/web                        # NAME=value lines (the secrets scope)
rdp domains shop/web
```

The address and token are kept in `~/.config/rdp/config.json`, or given as `RDP_URL` and
`RDP_TOKEN`. The address is https (http for localhost only); redirects are never followed, so
the token goes to the panel only.

`GET /api/v1/ping` (public) answers `{"pong": true}`.

## Admin routes (`/admin/api/x`)

- Writes (`POST`, `PUT`, `PATCH`, `DELETE`) need an allowed `Origin` and the session's CSRF token in
  `X-CSRF-Token` (`GET /admin/api/auth/csrf` returns it).
- Each route requires the permission shown. "member" is any signed-in user: the handler then checks
  the permission in the record's project (a role's, or a project role's). Errors from the panel are
  JSON with a stable `code`; a malformed path or query parameter (e.g. `?force=maybe`) gets a
  plain-text 400.

## Docker (`/docker`)

`{id}` is a container id or name.

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/info` | docker.view | engine version, OS, CPUs, memory, counts, swarm role |
| GET | `/containers` | docker.view | the containers the caller may see |
| GET | `/containers/{id}` | docker.view | inspect (secrets redacted) |
| POST | `/containers/{id}/{action}` | docker.manage | `start`, `stop`, `restart`, `kill`, `pause`, `unpause` |
| DELETE | `/containers/{id}?force=` | docker.manage | remove |
| GET | `/containers/{id}/logs?tail=&since=&timestamps=` | docker.logs | the last lines: `tail` up to 5,000 (default 500); reading stops after about 2 MiB of text; lines are cut to 16 KiB |
| GET (SSE) | `/containers/{id}/logs/follow?tail=&since=&timestamps=` | docker.logs | `lines` events: the tail, then new lines |
| GET (SSE) | `/containers/{id}/logs/follow/{tail}` | docker.logs | the same with the tail in the path (the panel's pages: their event streams take no query string) |
| GET (SSE) | `/containers/{id}/stats` | docker.view | `stats` events about every second |
| GET (WS) | `/containers/{id}/exec?cols=&rows=` | docker.exec | a terminal (80×24 without a size; the panel's page sends a resize once open): binary messages both ways; text `{"type":"resize","cols":80,"rows":24}`; the server sends `{"type":"exit"}` or `{"type":"idle"}` before closing |
| GET | `/images` | docker.view | images with tags, size, use |
| POST | `/images/pull` | docker.manage | `{"image": "nginx:1.27"}` → 202 with the queued action |
| DELETE | `/images/{tag or id}?force=` | docker.manage | remove (force needs system.manage) |
| DELETE | `/images?image=&force=` | docker.manage | the same, any name in the query (`registry:5000/app:1.2`: a path segment takes no `:` or `/`) |
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
| GET | `/overview` | projects.view_any, or a project member (their projects only) | the projects with their services (status, kind, primary domain) |
| GET | `/services/{id}` | services.view | a service's page: configuration (names of its environment variables, never values), domains, ports, mounts, pending changes, recent deployments (with deployments.view) |
| GET | `/deployments/{id}` | deployments.view | one deployment with its log |
| GET | `/services/{id}/stats` | services.view | the Overview's monitor: CPU and memory of each running task on this node, and their sum (two Docker stats samples, at most 3 s) |
| GET | `/services/{id}/errors` | services.view | the Overview's recent errors: failed tasks (exit code, refused, lost; not those a deployment stopped) and the last failed deployment with the end of its log (deployments.view) |
| GET (SSE) | `/services/{id}/logs/{tail}` | services.logs (in its project) or docker.logs | `lines` events: the service's logs (every task's), the last `tail` lines first; not a Compose service's |
| GET (WS) | `/services/{id}/exec/{container}` | services.exec (in its project) or docker.exec | a terminal in a running task container of the service (its full id), as `/docker/containers/{id}/exec` |
| POST | `/projects/{id}/services` | services.create | `{"name","kind","image"?,"database"?:{database,user,password,root_password}}`: an app (`kind` `app`, `image` required) or a database (`postgres`, `mysql`, `mariadb`, `mongo`, `redis`: credentials generated when left out); 422 on bad names, images, taken names; audited (`services.create`) |
| POST | `/services/{id}/deploy-key` | services.update | a new SSH deploy key (ed25519): `{"public_key"}`, the private half sealed; audited |
| POST | `/services/{id}/trigger` | services.deploy | a new deployment trigger URL, shown once: `{"url"}`; the old one stops working; audited |
| POST | `/services/{id}/basic-auth` | services.update | `{"username","password"}`: a user of its basic authentication (bcrypt; the same user replaced); audited; applied at once |
| DELETE | `/services/{id}/basic-auth/{user}` | services.update | removes one; audited |
| POST | `/scripts/{id}/run` | docker.exec | runs a script in a running container: `{"exit_code","output"}` (25 s at most); `{"command"}` must be the script as shown (409 otherwise); audited |
| GET | `/services/{id}/credentials` | services.secrets | a database's passwords and internal URL; audited first |
| GET | `/services/{id}/env` | services.secrets | `{"text","revision"}`: the environment as `.env` text, in clear; audited first (`services.env.read`, names only: no audit, no values) |
| PUT | `/services/{id}/env` | services.update | `{"text","revision"?}` replaces the environment (sealed); with the revision read, 409 `env_stale` when it changed meanwhile; 422 names the bad line; audited (`services.env.update`, names only) |

A database's page includes `database`: engine, internal host, port, user, database; never its
passwords. `GET /services/{id}/credentials` (services.secrets) answers `{"password",
"root_password","url"}`, audited first (`services.credentials.read`).

`/overview` and `/services/{id}` include each kind of record only with its resource's permission
(`services.view_any`, `service_domains.view_any`, `service_ports.view_any`, `service_mounts.view_any`).

A Compose or privileged service's diagnostics, logs and terminal are its administrators' only.

## Domains (`/apps`)

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/domains` | member (services.view_any and service_domains.view_any somewhere) | the Domains page: every domain the caller may see, where it leads and whether it is served; the panel's own domain, resolvers and certificates for system.manage |

## Firewall (`/apps`)

All `system.manage`; every change audited (`firewall.*`) and the host's changes logged as actions. See [firewall.md](firewall.md).

| Method | Path | |
|---|---|---|
| GET | `/firewall` | the page: `available` (installed panels only), `enabled`, `changed`, `drift`, `pending` (`txid`, `deadline`), `unsupported` (reasons), `ssh_ports`, `host` (nft, systemd, ufw, firewalld, nodes, `boot_failed`), `ports` (each listener and published port: `owner`, `state` `local`/`always_open`/`opened`/`closed`/`open`, its `openings`), `openings`, `blocked` |
| GET | `/firewall/preview` | the table the saved policy loads (`ruleset`) and the listening ports it closes (`closes`) |
| POST | `/firewall/openings` | `{"ports":"5432" or "8000-8100","protocol":"tcp"/"udp","target":"published"/"host","anywhere":bool,"sources_v4":"10.8.0.0/24, …","sources_v6":"…","note":…}`: sources canonical, each in its family; SSH, 80 and 443 (TCP) refused (always open) |
| PATCH | `/firewall/openings/{id}` | the same body, plus `enabled` |
| DELETE | `/firewall/openings/{id}` | |
| POST | `/firewall/blocked` | `{"cidr":"198.51.100.7" or "203.0.113.0/24","note":…}` |
| DELETE | `/firewall/blocked/{id}` | |
| POST | `/firewall/apply` | applies the saved policy: `{"txid","deadline"}`; refused while a change waits or when unsupported |
| POST | `/firewall/keep` | `{"txid"}`: keeps it (before its deadline, the host's table still the one it loaded) |
| POST | `/firewall/revert` | `{"txid"}`: back to the previous rules now |
| POST | `/firewall/disable` | turns it off (the table, the host's units and files removed) |

## Project members (`/apps`)

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/projects/{id}/members` | projects.members (in the project) | its members and the roles (`viewer`, `developer`, `admin`) |
| POST | `/projects/{id}/members` | same | `{"email","role"}`: a user of the panel given a role; audited |
| PATCH | `/projects/{id}/members/{user}` | same | `{"role"}`; the last admin changed only by whoever manages every project's members; audited |
| DELETE | `/projects/{id}/members/{user}` | same | removes one (the same rule); audited |
| GET | `/users?q=` | users.view_any | users whose email or name starts with `q` (2 characters at least; 10 at most) |

## API tokens (`/apps`)

Each user's own (Settings → API tokens); audited, never the token.

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/tokens` | member | the caller's live tokens, the scopes, `max_days` |
| POST | `/tokens` | member | `{"name","scopes","days"}`: `{"token","info"}`, the token shown once |
| DELETE | `/tokens/{id}` | member | revokes one of the caller's |

## Git (`/apps`)

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/git/providers` | system.manage | Settings → Git: the GitHub tokens (never their value) and the projects |
| POST | `/git/providers` | system.manage | `{"name","token","projects"}`: added once GitHub accepts the token, sealed; audited |
| PATCH | `/git/providers/{id}` | system.manage | renames it, replaces its token (checked first) or changes its projects; audited |
| DELETE | `/git/providers/{id}` | system.manage | removes it, unless a service clones with it; audited |
| POST | `/git/providers/{id}/test` | system.manage | asks GitHub who the token signs in as |
| GET | `/services/{id}/github` | services.update | the tokens the service's project was given (names, accounts) |
| GET | `/services/{id}/github/{provider}/repos` | services.update | the repositories that token reaches |
| GET | `/services/{id}/github/{provider}/branches?repo=owner/name` | services.update | a repository's branches |

## Templates (`/apps`)

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/templates` | services.create (a role's, or in a project) | the catalog: slug, name, description, tags, logo file |
| POST | `/templates/refresh` | services.create | fetch the catalog and templates again |
| GET | `/templates/{slug}` | services.create (a role's, or in a project) | name, description, instructions, links, `schema` (the form) |
| GET | `/templates/{slug}/logo` | services.create (a role's, or in a project) | the logo (image, `sandbox` CSP) |
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
| GET | `/actions` | projects.view_any, or a project member (their projects only) | the last 100 deployments (with deployments.view_any and deployments.view), backup runs (database_backups.view_any) and Docker actions (docker.view; own unless system.manage): `{type, id, what, status, error, detail?, target?: {project, service}, created_at, finished_at}` |

## Notifications (`/apps`)

Channels are written through the `notification_channels` resource.

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/notification-channels` | notification_channels.view_any | the channels (never their secret: `secret_set`) |
| POST | `/notification-channels/{id}/test` | notification_channels.update | sends a test notification now: `{"ok","error"?}` |

## Certificates (`/apps`)

Resolvers are written through the `cert_resolvers` resource (their token sealed, never shown).

| Method | Path | Permission | |
|---|---|---|---|
| GET | `/traefik/resolvers` | system.manage | the resolvers (`has_token`, `in_use`), the DNS providers, the default resolver, and whether Traefik runs them (`applied`: `null` without a Traefik service) |
| POST | `/traefik/apply` | system.manage | gives Traefik the resolvers (it restarts); logged as an action |
| GET | `/panel` | system.manage | the panel's domain, Let's Encrypt email (from Traefik's arguments), apps' domain and address, as the installer set them |
| GET / PUT | `/home` | system.manage | the homepage at `/` (Settings → Panel): `{"enabled"}`; the change audited |
| GET / PUT | `/branding` | system.manage | Settings → Branding: `{"name","primary"}` (empty: the defaults); audited |
| PUT / DELETE | `/branding/images/{kind}` | system.manage | `logo`, `logo-dark` or `favicon`: `{"mime","data"}` (base64) / removed; audited |
| GET | `/tunnel` | system.manage | Settings → Cloudflare Tunnel: its state |
| POST | `/tunnel/enable` | system.manage | `{"token"?}` (none: the one kept): cloudflared run, Traefik changed; audited (never the token) |
| POST | `/tunnel/disable` | system.manage | stops it; audited |
| GET | `/system/status` | system.manage | Settings → Panel: version, installed, image, hostname, public IP, whether it restarts and reboots |
| GET (SSE) | `/system/logs/{tail}` | system.manage | `lines` events: the panel's log (its service's when installed; else the lines this process kept) |
| POST | `/system/restart` | system.manage | restarts the panel's service (installed only); logged and audited first |
| POST | `/system/reboot` | system.manage | `{"confirm"}` (the server's hostname): reboots the server (installed only); logged first |
| POST | `/system/ip` | system.manage | finds the server's public address again; logged as an action |
| POST | `/system/prune/{kind}` | system.manage | `containers`, `images`, `build_cache` or `volumes` (never the services' and Compose's): `{"report"}`; logged as an action, audited, notified |
| POST | `/panel/check` | system.manage | `{"domain","email","apps_domain"}`: `{"errors","warnings","dns","command"}` — checked (names, a service's domain, DNS records) and the installer command applying them |
| GET | `/system/version` | system.manage | Settings → Version: the panel (version, commit, image, digest, channel `release`/`development`/`source`), the last check (`checked_at`, `latest`, `error`), the update (`null` but on a release older than the newest), its release notes, the installer command for it, the components (Core, Traefik, MySQL, cloudflared, Docker, OS) and the history (version changes, restarts) |
| GET | `/domains/dns?host=` | service_domains.view_any | where a host leads: `{"host", "via": "server" \| "cloudflare" \| "elsewhere" \| "none", "tunnel"}` (answers kept 5 minutes); the Domains page's rows carry `via` too |
| POST | `/system/version/check` | system.manage | checks now (the registry's tags, GitHub's notes); 429 within a minute of the last; then as `GET /system/version` |
| GET | `/system/version/notice` | system.manage | the Dashboard's notice for the caller: `{"notice": {"version"} \| null}` (none once dismissed for that version) |
| POST | `/system/version/update` | system.manage | `{"version": "X.Y.Z"}` (the newest release, newer than this one): the update recorded (`panel.update`, audited) and run in the background — new deployments, backups and restores answer 409 `panel_updating` meanwhile; 409 while one runs; 422 from source or another version |
| POST | `/system/version/dismiss` | system.manage | `{"version": "X.Y.Z"}`: the caller's notice of that version hidden |
| GET | `/system/cleanup` | system.manage | what removing the panel's old versions would remove: `{"cleanup": {containers, images: [{id, tags, size}], space}}` |
| POST | `/system/cleanup` | system.manage | removes them (old tasks' stopped containers of the panel's services, then their images; the running and previous versions kept, never forced); logged as an action |
| GET | `/traefik/status` | system.manage | Traefik's image, published ports, tasks (state, error), last update; the custom middlewares' YAML (never its arguments) |
| POST | `/traefik/restart` | system.manage | restarts Traefik (its tasks replaced); logged as an action |
| PUT | `/traefik/middlewares` | system.manage | `{"yaml"}`: the custom middlewares, checked, written for Traefik (no restart): `{"middlewares": ["custom-x@file", …]}` |
| GET (SSE) | `/traefik/logs/{tail}` | system.manage | `lines` events: Traefik's output, the last `tail` lines (5000 at most) first |
| GET | `/certificates` | system.manage | the uploaded certificates (never their keys): names, expiry, `days_left`, `in_use` |
| POST | `/certificates` | system.manage | `{"name"?, "chain", "key"}` (PEM): checked, the key sealed, served by Traefik at once: `{"id","self_signed"}` |
| PUT | `/certificates/{id}` | system.manage | the same, replacing it (a renewed one: it must cover its domains' hosts; no deployment needed) |
| DELETE | `/certificates/{id}` | system.manage | removes one no domain and no deployed route uses |

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
