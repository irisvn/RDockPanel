# Importing existing containers

A container started with `docker run` or `docker compose` can be brought into a project: it becomes
an ordinary service of the panel (deploy, rollback, environment, ports, volumes, domains, backups,
logs, terminal, members). It is **recreated**: the panel reads the container and the ones it needs
with it, plans the move, stops the originals, copies their data, deploys services of its own and keeps
the originals so that the whole thing can be undone. [Features](features.md) ·
[Architecture](architecture.md#importing-existing-containers).

**In the interface:** the Dashboard and a project have **Import containers**; Docker → Containers has
**Bring into the panel…** in a row's menu; a service's Storage tab has **Move all** when it uses a folder in
place. Each opens or leads to the import's page (`/p/dashboard/_import/{id}`: its plan, what it loses, its
state and the buttons of that state). A project's name cannot start with `_`, so the path never clashes.

Only users with `system.manage` import (permission checks on the project are made when it starts).
There is no new permission.

## What happens

1. **Read.** `POST /imports` names a container or a Compose project. The job `import.discover` reads
   Docker (read only): every container's inspect, the images, the volumes, the Compose files (through a
   helper container that is created and never started, with the file mounted read only). It finds the
   **closure**: the containers that must move together (a writable volume or folder they share, a link,
   a name one calls the other by in its environment or command; at most 50). The result is a **plan**:
   names, ports and findings, never a value of an environment variable.
2. **Plan.** Each container is `bring`, `leave` or `blocked`. A finding is `BLOCK` (cannot be imported
   until the cause is gone), `CONFIRM`, `WARN` or `INFO`. Calls by name between containers are soft
   edges: leaving one side behind needs `confirm: true` and lists what breaks. Writable data
   shared by containers is a hard edge: they go together, or none does.
3. **Start.** Typing the project's name starts it. The plan is read again and its fingerprint must be the one
   the admin saw (else `409 plan_stale`). In one write the project, a service for each container, the
   import's items, volumes and claims (one container or volume belongs to one import), the audit entry
   `imports.start` and the first step are recorded.
4. **Steps** (a job each, under a lease that a long copy or deployment renews): stop the originals
   (restart policy off first), wait for them, check there is room (the volume's size and a margin of 10% or
   2 GB), copy each named volume with a helper (`cp -a`, no network, no capability but `CHOWN`,
   `DAC_OVERRIDE`, `FOWNER`), make the symlink of each folder, rename the Compose file to
   `<name>.rdockpanel-imported` (only when every container of that project goes), deploy the services a level at a time
   (a service after what it calls), observe them for 60 seconds (a failure that happens while they are watched counts, not the failures of
   their start), then `active`.
5. **Finish** marks the import `finishing` (from then it cannot be undone), removes the originals (not
   their volumes), then `finished`. A removal Docker refused leaves it `finishing`: **Finish again**. The
   original volumes stay until the admin releases them (a volume prune leaves them).

### Undo

Until `finishing`, **Undo** destroys the new services (each one, deployed or not: a first deployment that
failed may have made it in Swarm already), waits until their task containers are gone,
puts the Compose file back, restores the restart policies and starts the originals again. A failure
before traffic could reach a new service (a port, a domain or a tunnel) undoes by itself; after, the import waits
in `needs_attention` with **Retry**, **Continue** (after a database stopped by force, exit 137),
**Accept** (when every service runs, never an undo stopped on its way) or **Undo**. A Compose file is
recorded before it is renamed, and put back only when the renamed copy is the one read (its SHA-256) and
nothing has taken its name since. The copied volumes are kept when undone (they may
hold what was written since).

## What it maps, and what it refuses

| Container | Service |
|---|---|
| image | the digest of the image that runs (`repo@sha256:…`); an image only this machine has is tagged `rdp.invalid/<project>/<service>:<12 hex>` (never pulled, never removed); the service is the administrators' (only `system.manage` names such an image), and its tasks run on this machine's Swarm node (`node.id`), as do those of a service with a folder used in place |
| environment | names and values sealed like any service's; pairs equal to the image's own are left out |
| named volume | a volume of the panel's, `rdp_<project>_<service>_<name>`, copied |
| host folder | used in place through a symlink the panel makes (`link` mount); the service is `admin_only` until the folder is moved |
| health check | its command (a shell line), interval, retries, timeout and start period; a password in it moves to `RDP_SECRET_HC_<n>` (sealed) and the check reads `"$RDP_SECRET_HC_<n>"`. Secrets are told by the program: `-p<value>` is a password for mysql/mysqladmin/mariadb/mongo(sh) only (a port elsewhere), `--password`, `--requirepass`, `-a` of redis-cli, `-u user:pass`, `PGPASSWORD=…`; `$NAME` is a reference, never a secret. One that cannot be cut (inside a URL, a nested script) blocks the container until the admin drops the check or types one (without a secret: `-p"$NAME"`). `NONE` or a dropped check: off (the image's own does not run either) |
| capabilities, sysctls, groups, CPUs | carried (`cap_add`, `cap_drop`, `sysctls`, `groups`, `cpu_limit_millis`) |
| shared memory, ulimits, tmpfs, devices, extra hosts, DNS, security options, read-only root, stop signal and grace, working directory, log driver, `init: false` | said in the plan (`setting_lost`), not carried |
| command | as it was; a secret in it (`--requirepass x`, `--password=x`) moves to an environment variable `RDP_SECRET_<n>` and the command becomes `sh -c 'exec <entrypoint> <command> "$RDP_SECRET_<n>"'` (confirmed: the image needs a `/bin/sh`). Only when the image's entrypoint runs the command it is given (none, `docker-entrypoint.sh`, `tini --`, `dumb-init --`); with a program as entrypoint the import is blocked. A secret env name is told by its words (`DB_PASSWORD`, `GITHUB_TOKEN`), not by a part of a word (`PASSENGER_APP_ENV`, the shell's `PWD`) |
| user | a container that ran as another user than its image's (a tunnel as root) keeps it: `services.user`, shown on the service, and the service is `admin_only` |
| Cloudflare tunnel (`cloudflared` with a token in `TUNNEL_TOKEN` or `--token`) | a service, or, chosen in its details, **a tunnel of the panel** (Settings → Cloudflare Tunnel), named after the container (`tunnel`, `tunnel-2` when the name is taken): no service is made, the panel runs `rdp-cloudflared-<name>` with that token when it comes to this container's level, and an undo removes that tunnel (only the ones the import made). Several cloudflared (several accounts) become several tunnels. A token that a tunnel of the panel has already, or that two containers share, is refused (one cloudflared per token): the container can still come as a service. Every public hostname must then point to Traefik (Settings shows where) and each service needs a domain: said in the confirmation |
| read-only mount (`:ro`) | stays read only |
| Traefik middlewares of a router | **basicAuth** (users on the container's labels): its users and their hashes become the service's basic auth (read from Docker when the import starts, never kept in the plan; bcrypt, `$apr1$` and `{SHA}` hashes). A domain behind any other protection (forwardAuth, an allow list, digestAuth, a chain, a middleware defined elsewhere `@file`) is left out and cannot be ticked: protect the service in the panel, then add the domain. Middlewares that protect nothing (redirectScheme, headers, compress…) are said and not carried |
| ports | published as they were; loopback ones and the panel's reserved ones are left out |
| Traefik labels (`traefik.http.routers.*.rule` with `Host()` and `PathPrefix()`), `VIRTUAL_HOST` | the service's domains, with the port of the label (`VIRTUAL_PORT`; else the only exposed port, else 80), added with the service when the import starts. The plan lists them ticked; unticked in the container's details, one is left out. A host and path the panel already serves (any service), or that a container before it in the plan has, is left out and said, to confirm (`domain_taken`); one the panel's rules refuse (path, the panel's own host, its certificate) is left out and said (`domain_invalid`); an undo removes the domains with the service |

Blocked: a bind mount of a place the panel never gives a service (`/`, the Docker socket, `/proc`, `/sys`,
`/dev`, `/var/lib/docker`, the installer's folder: the rules of bind mounts; checked again when the link is made,
at every deployment and by Move all), paused or dead containers, `--rm`, privileged, host network, `network_mode: container:`, an entrypoint
different from the image's, a volume with a driver or options, an environment variable that
is not a valid name or holds `$(…)`, more than 100 variables or a value over 16 KiB, a secret in the
command line that cannot be cut out of its argument (a URL's password), two containers
sharing a Docker volume (in the panel a volume belongs to one service), and the panel's own objects.
a secret in a container's health check, and two containers sharing a Docker volume (in the panel a
volume belongs to one service; two that only read the same one each get a copy).
Said and lost: a port published on one address only (the panel publishes it on every address), hostname, extra aliases and networks, ulimits, log config, the restart policy
(the panel always restarts).

## Move all, and clean the old folder

A service with a folder in use in place has these buttons (`POST /services/{id}/move-data`,
`/move-data/undo`, `/clean-old-folder`): **Move all** stops it, copies the folder into a volume, switches the
mount in one write with its audit entry, deploys and observes; **Undo** puts the link back (what was
written to the volume stays there); **Clean the old folder** (the folder's name typed) deletes it.

- **Move all** is refused when another service or container (stopped ones too) uses the folder, or a folder
  inside or above it: a copy would split the data. It is checked again once the service is stopped, the room
  (the folder's size and a margin of 10% or 2 GB) is checked, and the volume is recorded before it is made
  (an undo removes it).
- **Clean** first marks the folder `cleaning` with its audit entry, under the move's lock: from then the
  move cannot be undone, and a clean that failed is run again. One helper mounts the folder's parent,
  refuses a symlink (and a folder swapped while it looks), empties the folder from inside it (`find -delete`
  never follows a link) and removes it. It is refused while the import that brought the service in can still
  be undone, while the service uses the folder again (a link or a bind), and while another service or
  container uses it or a folder inside it. A symlink in a folder *above* it is resolved by Docker: keep the
  folders above it writable by root only.

## Before it starts

The plan says the data to copy (the named volumes' sizes, Docker's `/system/df`), the free room where Docker
keeps volumes (a helper's `df` of a volume to copy) and the downtime at most: the stops one after the other (up
to 120 s for a container that writes data, 30 s for another), the copy at ~100 MB/s, ~30 s a level to deploy, 60 s
watched. An import whose data and margin (10%, at least 2 GiB) do not
fit is refused (`409 no_room`); the copy checks again. A port the panel keeps (`[panel] reserved_ports`) or one
on the loopback can be published on another (`PATCH plan {"ports": {container: {"80/tcp": 8081}}}`: valid, not
kept, not another service's nor the plan's). A Compose project not renamed (some of its containers stay) says on
its brought containers that `docker compose up` would start the originals again.

The page shows what each step did (`container_import_events`: stopped, copied, linked, deployed, watched,
undone; names only, once per step, at most 500).

## Safety

- Nothing runs in a request: the panel reads and writes through jobs under a lease that expires if a job dies
  (`import-tick`, every minute, queues the step again; a step whose lease is taken by another run stops,
  and its helper is removed). A reading that ran out of time is marked failed after 10 minutes.
- While an import runs, or waits for the admin, **nothing else touches its services**: deployments,
  backups and restores answer `409 import_in_progress`, and the originals, their volumes and `prune`
  cannot be changed from the Docker pages. The panel does not update while an import runs.
- `import-tick` also **watches the originals**: one that runs again (`docker compose up`, a person) is
  stopped again until the import is deleted or the admin says to stop watching. The panel can detect
  it, not prevent it.
- Helpers are created with `CapDrop ALL`, no network, a read-only root, `no-new-privileges`, never
  privileged, never auto-removed, with a fixed name, and labelled `rdockpanel.system` and
  `rdockpanel.import=<id>`.
- Every secret goes through one function (`services/secret_scan_service.rs`); the plan, the answers, the
  audit entries and the logs carry names, never values.

## Routes

All under `/admin/api/x`, `system.manage`:

| Method | Path | |
|---|---|---|
| GET | `/imports/sources` | the containers, by Compose project (25 groups a page, `?search=` by project or container name); names and states only |
| POST | `/imports` | `{source_kind: container\|project, source_ref, project_name?}`: 202 |
| GET | `/imports/{id}` | the state, the error and the plan |
| PATCH | `/imports/{id}/plan` | `{choices?: {container_id: bring\|leave}, project_name?, confirm?, tunnel?, domains?}`; `409 plan_stale` when the plan changed meanwhile |
| POST | `/imports/{id}/rescan` | read again |
| POST | `/imports/{id}/start` | `{confirm: <project name>}`: 202 |
| POST | `/imports/{id}/undo`, `retry`, `continue`, `accept`, `finish`, `stop-originals`, `stop-watching` | |
| POST | `/imports/{id}/volumes/{vid}/release` | `{remove?}`: lets go of an original volume |
| DELETE | `/imports/{id}` | an import that did not start |
| POST | `/services/{id}/move-data`, `/services/{id}/move-data/undo`, `/services/{id}/clean-old-folder` | |

## Settings

`[panel] helper_image` (config/panel.toml): an image with `cp` and `sh` for the helpers (an installed
panel uses its own image; a panel run from source needs one). The folder of the symlinks is
`<state_dir>/links` on a Mac and `/var/lib/rdockpanel/links` on Linux. The queue `import` is served by
the shared worker pool (`workers = 4` in `config/queue.toml`).

## Tables

`container_imports` (RustCMS has its own `imports` table, for CSV files), `container_import_items`,
`container_import_volumes`, `container_import_claims` (unique container and unique volume),
`container_import_files`; the columns `services.admin_only`, `services.run_user`,
`service_mounts.link_target`, `container_imports.flags`, `container_import_items.tunnel_id` and
`container_import_volumes.cleaning_at`. Each migration's `down` refuses while it would lose what an import
recorded.

## Not done yet

The streamed log of an import (the page looks again every 2 seconds), databases as database services (they map to apps), a
partial import of a Compose project does not edit its file (the watch stops what runs again), the Docker
network guard for the originals' networks, the volume's size and the estimated downtime in the plan's
overview, and the plan's choice of a domain or another port for a port the panel keeps.
