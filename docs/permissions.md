# Permissions

Permissions live in the `admin` guard and are granted through roles (System → Roles). Run
`rustcms permission:sync` after an upgrade to create new ones. A `super_admin` has all of them.

| Permission | Allows |
|---|---|
| `server.view` | the Dashboard's server metrics: live CPU, memory, network and disk |
| `docker.view` | the Containers, Images, Volumes and Networks pages; container inspect and live stats; an action's progress (own actions only) |
| `docker.logs` | container logs (snapshot and live follow) |
| `docker.manage` | start, stop, restart, pause, unpause, kill and remove containers; pull and remove images; prune; create and remove volumes and networks |
| `docker.exec` | the container terminal (a root shell in most images: grant carefully) |
| `system.manage` | the panel's own objects (labelled `rdockpanel.system=true`): see them, inspect them, read their logs, open a terminal in them, act on them; force-remove images in use; read every user's actions |

## Projects and services (Phase 2)

The pages of the sidebar need: the Dashboard `projects.view_any` (projects, services and their
tabs) or `server.view` (its server metrics only); Actions `projects.view_any`; Settings
`storage_providers.view_any` (its Backups tab), `notification_channels.view_any` (Notifications) or
`system.manage` (Panel, Traefik, Certificates): each tab shows with its own. The resources behind them have no
sidebar entry, so give a role that manages services `projects.view_any` as well (their routes,
`/admin/r/<slug>`, still answer to the resource's own permissions).

The resources use the panel's standard permissions, one set per resource
(`view_any`, `view`, `create`, `update`, `delete`): `projects.*`, `services.*`,
`service_domains.*`, `service_ports.*`, `service_mounts.*`, and `deployments.view_any` /
`deployments.view` (deployments are written by the panel only). Besides:

- `services.deploy` runs a service's Deploy, Start, Stop, Restart and Republish routes; a
  deployment's Rollback needs `services.deploy` and `deployments.view` (to a deployment that
  succeeded running a recorded image: not a Stop or a Destroy);
- `services.destroy` removes a service from Swarm (Destroy; its volumes are kept);
- a service's deployments (their log, error and image), on its page too, need `deployments.view`;
- environment variables and registry credentials are fields of the service (`services.update`);
  reading environment values in clear (the Environment tab) needs `services.secrets`, and each read
  is audited; the registry password is never shown;

- database backups: `storage_providers.*` (Settings), `database_backups.*` (a database's
  configurations; their tab needs `database_backups.view_any`), `backups.run` (run one now) and
  `backups.restore` (restore one, replacing the database's data; lists a provider's files);

- notifications: `notification_channels.*` (Settings);
- certificate resolvers (Settings → Certificates) and their "Apply" (it restarts Traefik): `system.manage`
  only. Choosing a resolver on a domain, and changing or removing a domain that has one, need
  `system.manage` too (a resolver asks for a certificate for any name it is given). No domain may use
  the panel's own host;
- uploaded certificates (Settings → Certificates, Uploaded certificates: upload, replace, delete): `system.manage`
  only; choosing one on a domain too;
- Traefik (Settings → Traefik: its state, restart, live logs, custom middlewares): `system.manage` only;
- the panel's domain, Let's Encrypt email and apps' domain (Settings → Panel: shown, checked, the
  installer command given): `system.manage` only;
- adding Linux capabilities, sysctls or supplemental groups to a service needs `system.manage`
  (dropping capabilities does not). Such a **privileged** service (like a Compose service) is then
  the panel's administrators' only: editing it and every record of it (domains, ports, mounts,
  redirects, scripts, backups; none moves to another service), its environment, basic
  authentication and deploy key, deploying or rolling it back (even to a plain deployment),
  making its trigger URL or GitHub webhook (deliveries to a privileged service are refused),
  running its scripts or backups, restoring into it (a restore asked for before it became
  privileged is refused when it runs). Its containers stay under `docker.manage` and
  `docker.exec` (root in most images anyway): grant those carefully;
- redirects: `service_redirects.*`; basic authentication users: `services.update`; scripts:
  `service_scripts.*` to write them, `docker.exec` to run them (a command in the container);

- a **host directory** (bind mount) needs `system.manage`, and a source under one of the
  `[panel] bind_roots` of `config/panel.toml` (none by default), resolved with its symlinks; `/`, `/proc`,
  `/sys`, `/dev`, `/boot`, `/var/lib/docker`, the Docker socket, `/etc/rdockpanel`, `/run/secrets`,
  and any directory above them (`/run`, `/etc`, `/var`), are always refused (keep the panel's other
  files, such as a checkout with its `.env`, out of the roots);
- a service, once deployed (from its first deployment on, even a failed one) and until a successful
  Destroy, keeps its name and project and cannot be deleted; a project with services cannot be
  deleted, nor renamed while one of them is deployed;
- the ports in `[panel] reserved_ports` (the proxy's and the panel's) cannot be published.

## Project members

A user needs no role to work on a project: a project's admin (or anyone whose roles grant
`projects.members`) makes them a member, with one of three roles, on the project's page
(**Members**):

| Role | May |
|---|---|
| viewer | see the project, its services and their records, deployments, backups, logs |
| developer | as a viewer, and change, add, deploy and roll back its services; their domains, ports, mounts, redirects, scripts and backup schedules; read their environment (`services.secrets`); open their terminal (`services.exec`); run a backup |
| admin | as a developer, and destroy and delete services, restore a database (its own backups only), rename the project, choose its members |

A role of the user's (System → Roles) still applies to every project: a member's permissions in a
project are their roles' plus their project role's. A member sees their projects only, through
every route: the Dashboard and Domains, the service pages, the resources' API (lists, records,
actions), deployment logs, terminals and logs (closed when the membership ends), backups and
templates. `system.manage` is never a project's to give: a Compose or privileged service stays its
administrators'. A project keeps one admin: only someone whose roles grant `projects.members`
changes or removes its last one. Members get `apps.access` (the apps pages, nothing else) while
they are a member of some project.

Backups: a member runs a project's backup schedules (developer) and restores its service's own
backups (admin: chosen among that service's backups, without browsing the provider). Creating a
schedule chooses a storage provider, whose settings stay a role's (`storage_providers.view`).

`services.logs` and `services.exec` also exist as permissions of their own: a role with them reads
a service's logs and opens its terminal from the service's page, without `docker.*` (the Docker
pages).

## The panel's own objects

Objects labelled `rdockpanel.system=true` are the panel's own. The installer (Phase 2, not written
yet) will put that label on the panel, its MySQL and Traefik (services, containers, volumes, networks,
secrets); until then only the development Traefik (`scripts/dev-swarm.sh` labels its service and
containers) and objects you label yourself are protected (the development MySQL from
`docker-compose.yml` is not). Without `system.manage`:

- they are left out of every list;
- inspect, logs, stats, terminal and every change answer 403;
- an image that one of them runs cannot be removed;
- prune always skips them, whoever runs it.

## Checks happen twice

Each route declares its permission and is refused before the request is read. Live streams and
terminals check again every 30 s (sockets every 10 s) and stop within seconds of a lost permission,
a sign-out or a password change. Terminal commands (resize) are authorized when they run.
