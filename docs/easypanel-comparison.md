# RDockPanel and EasyPanel, feature by feature

RDockPanel aims at EasyPanel's interface and features (from easypanel.io/docs, October 2026), all
free. **Done** works in the panel; **partly** works with the limits said; **missing** is not there.

## App service

| EasyPanel | RDockPanel | |
|---|---|---|
| Overview: Deploy, Start, Stop, Restart, Open | Done | always visible in the service's header |
| Overview: Logs, Shell | Done | open the running container's live logs or terminal |
| Overview: metrics, notes | Done | CPU and memory of each task (sparklines); notes for the team |
| Overview: Force Rebuild, recent errors | Partly | Deploy rebuilds a built service; errors are in the deployment's log |
| Source: Docker image (private registry) | Done | pinned to its digest |
| Source: Git, GitHub | Done | https or ssh with a generated deploy key; branch, tag or commit; build path |
| Source: Dockerfile | Done | written in the panel |
| Source: Upload | Missing | |
| Build: Dockerfile, Nixpacks | Done | BuildKit; Nixpacks in the panel's image |
| Build: Railpack, Buildpacks | Missing | |
| Deployments: history, live log, rollback, trigger URL | Done | |
| Auto deploy (GitHub) | Done | signed webhook; pushes during a deployment are kept |
| Environment (`.env` text) | Done | encrypted; values shown only with `services.secrets` (audited) |
| Domains: host, path, port, HTTPS, primary, generated domains | Done | Let's Encrypt in the installer |
| Domains: wildcard, custom middlewares, certificate resolver per domain | Missing | one resolver for the panel |
| Redirects | Done | Go regular expressions, permanent or not, on or off |
| Security: basic authentication | Done | bcrypt; applies at the next deployment |
| Scripts | Partly | run non-interactively, at most 25 s, output shown (EasyPanel runs them in a shell) |
| Resources: CPU and memory reservations and limits | Done | in Advanced |
| Maintenance mode: title, subtitle | Done | the panel serves the page (503) |
| Maintenance: white-label logo and CSS | Missing | a paid EasyPanel feature |
| Storage: volume, bind (system managers), file mounts | Done | file contents encrypted |
| Storage: volume backups | Done | scheduled, retention, restore |
| Advanced: ports, replicas, zero downtime, command, health check | Done | |
| Advanced: Tini, capabilities, sysctls, supplemental groups | Done | Tini always; adding capabilities, sysctls or groups needs `system.manage` |

## Databases (PostgreSQL, MySQL, MariaDB, MongoDB, Redis)

| EasyPanel | RDockPanel | |
|---|---|---|
| Created with a name only (credentials generated) | Done | |
| Credentials: user, password, internal host and URL, copy buttons | Done | passwords with `services.secrets` (audited) |
| Expose (publish the port) | Done | as a port, in Advanced |
| Backups: schedule, retention, provider, path, Run now, restore, log | Done | Redis keeps its append-only file (as EasyPanel) |
| Shell with `psql` and co., PgWeb/DbGate | Partly | the container's terminal; no web client |

## Compose, templates, panel

| EasyPanel | RDockPanel | |
|---|---|---|
| Compose service: inline or Git, domains to a container, `.env` | Done | for the panel's administrators (`system.manage`) |
| Templates (800+), their form, deploy at once | Done | fetched from EasyPanel's repository; 829 of 843 ran in October 2026 |
| Create from Schema | Done | |
| Dashboard: server metrics, projects and services | Done | |
| Actions page | Done | deployments, backups, Docker actions |
| Monitor (server, per service) | Done | server page; per service on its Overview |
| Settings: storage providers (Local, S3) | Done | with a Test button |
| Settings: notifications (Discord, Slack, Telegram, email, webhook) | Done | deployments, backups, a full disk; Send test |
| Settings: panel domain, Let's Encrypt email | Partly | set by the installer (run it again to change them) |
| Users and roles | Done | RustCMS roles and fine-grained permissions |
| Quick search (⌘K) | Done | |
| Cluster (several nodes), Cloudflare Tunnel, white-label | Missing | later (see the [roadmap](roadmap.md)) |
