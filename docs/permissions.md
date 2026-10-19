# Permissions

Permissions live in the `admin` guard and are granted through roles (System → Roles). Run
`rustcms permission:sync` after an upgrade to create new ones. A `super_admin` has all of them.

| Permission | Allows |
|---|---|
| `server.view` | the Monitor page: host facts and live CPU, memory, network and disk |
| `docker.view` | the Containers, Images, Volumes and Networks pages; container inspect and live stats; an action's progress (own actions only) |
| `docker.logs` | container logs (snapshot and live follow) |
| `docker.manage` | start, stop, restart, pause, unpause, kill and remove containers; pull and remove images; prune; create and remove volumes and networks |
| `docker.exec` | the container terminal (a root shell in most images: grant carefully) |
| `system.manage` | the panel's own objects (labelled `rdockpanel.system=true`): see them, inspect them, read their logs, open a terminal in them, act on them; force-remove images in use; read every user's actions |

## Projects and services (Phase 2)

The pages of the sidebar need: the Dashboard (projects, services and their tabs) `projects.view_any`;
Actions `projects.view_any`; Monitor `server.view`; Settings `storage_providers.view_any` or
`notification_channels.view_any` (each section shows with its own). The resources behind them have no
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
- uploaded certificates (Settings → Uploaded certificates: upload, replace, delete): `system.manage`
  only; choosing one on a domain too;
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
