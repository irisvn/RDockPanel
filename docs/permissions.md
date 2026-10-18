# Permissions

Permissions live in the `admin` guard and are granted through roles (System → Roles). Run
`rustcms permission:sync` after an upgrade to create new ones. A `super_admin` has all of them.

| Permission | Allows |
|---|---|
| `server.view` | the Server page: host facts and live CPU, memory, network and disk |
| `docker.view` | the Containers, Images, Volumes and Networks pages; container inspect and live stats; an action's progress (own actions only) |
| `docker.logs` | container logs (snapshot and live follow) |
| `docker.manage` | start, stop, restart, pause, unpause, kill and remove containers; pull and remove images; prune; create and remove volumes and networks |
| `docker.exec` | the container terminal (a root shell in most images: grant carefully) |
| `system.manage` | the panel's own objects (labelled `rdockpanel.system=true`): see them, inspect them, read their logs, open a terminal in them, act on them; force-remove images in use; read every user's actions |

## Projects and services (Phase 2)

The Deploy group's resources use the panel's standard permissions, one set per resource
(`view_any`, `view`, `create`, `update`, `delete`): `projects.*`, `services.*`,
`service_domains.*`, `service_ports.*`, `service_mounts.*`, and `deployments.view_any` /
`deployments.view` (deployments are written by the panel only). Besides:

- `services.deploy` runs a service's Deploy, Start, Stop, Restart and Republish routes, and a
  deployment's Rollback
  (to a deployment that succeeded running a recorded image: not a Stop or a Destroy);
- `services.destroy` removes a service from Swarm (Destroy; its volumes are kept);
- a service's deployments (their log, error and image), on its page too, need `deployments.view`;

- a **host directory** (bind mount) needs `system.manage`, and a source under one of the
  `[panel] bind_roots` of `config/panel.toml` (none by default), resolved with its symlinks; `/`, the
  Docker socket, `/proc`, `/sys`, `/dev`, Docker's and the panel's own files, and any directory above
  them (`/run`, `/etc`, `/var`), are always refused;
- a service, once deployed (from its first deployment on, even a failed one), keeps its name and
  project, and cannot be deleted before it is destroyed; a project with services cannot be deleted,
  nor renamed once one of them was deployed;
- the ports in `[panel] reserved_ports` (the proxy's and the panel's) cannot be published.

## The panel's own objects

Objects labelled `rdockpanel.system=true` are the panel's own. The installer (Phase 2, not written
yet) will put that label on the panel, its MySQL and Traefik (services, containers, volumes, networks,
secrets); until then only objects you label yourself are protected (the development MySQL from
`docker-compose.yml` is not). Without `system.manage`:

- they are left out of every list;
- inspect, logs, stats, terminal and every change answer 403;
- an image that one of them runs cannot be removed;
- prune always skips them, whoever runs it.

## Checks happen twice

Each route declares its permission and is refused before the request is read. Live streams and
terminals check again every 30 s (sockets every 10 s) and stop within seconds of a lost permission,
a sign-out or a password change. Terminal commands (resize) are authorized when they run.
