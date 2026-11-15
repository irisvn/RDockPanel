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
5. **Finish** removes the originals (not their volumes). The original volumes stay until the admin
   releases them.

### Undo

Until `finished`, **Undo** destroys the new services, waits until their task containers are gone,
puts the Compose file back, restores the restart policies and starts the originals again. A failure
before traffic could reach a new service (a port, a domain or a tunnel) undoes by itself; after, the import waits
in `needs_attention` with **Retry**, **Continue** (after a database stopped by force, exit 137),
**Accept** (when every service runs) or **Undo**. The copied volumes are kept when undone (they may
hold what was written since).

## What it maps, and what it refuses

| Container | Service |
|---|---|
| image | the digest of the image that runs (`repo@sha256:…`); an image only this machine has is tagged `rdp.invalid/<project>/<service>:<12 hex>` (never pulled, never removed) and the service is `admin_only` |
| environment | names and values sealed like any service's; pairs equal to the image's own are left out |
| named volume | a volume of the panel's, `rdp_<project>_<service>_<name>`, copied |
| host folder | used in place through a symlink the panel makes (`link` mount); the service is `admin_only` until the folder is moved |
| ports | published as they were; loopback ones and the panel's reserved ones are left out |

Blocked: paused or dead containers, `--rm`, privileged, host network, `network_mode: container:`, a user or
entrypoint different from the image's, a volume with a driver or options, an environment variable that
is not a valid name or holds `$(…)`, more than 100 variables or a value over 16 KiB, a secret in the
command line (`--requirepass x`: make the container read it from the environment first), two containers
sharing a Docker volume (in the panel a volume belongs to one service), and the panel's own objects.
a secret in a container's health check, and two containers sharing a Docker volume (in the panel a
volume belongs to one service; two that only read the same one each get a copy).
Said and lost: a port published on one address only (the panel publishes it on every address), hostname, extra aliases and networks, ulimits, log config, the restart policy
(the panel always restarts).

## Move all, and clean the old folder

A service with a folder in use in place has these buttons (`POST /services/{id}/move-data`,
`/move-data/undo`, `/clean-old-folder`): **Move all** stops it, copies the folder into a volume, switches the
mount in one write with its audit entry, deploys and observes; **Undo** puts the link back (what was
written to the volume stays there); **Clean the old folder** (the folder's name typed) deletes it with
helpers that mount exactly that folder, after the rules are checked again against the record. It is
refused while the import that brought the service in can still be undone (its original runs on that folder)
and while the service uses the folder again.

## Safety

- Nothing runs in a request: the panel reads and writes through jobs under a lease that expires if a job dies
  (`import-tick`, every minute, queues the step again).
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
| GET | `/imports/sources` | the containers, by Compose project (25 groups a page); names and states only |
| POST | `/imports` | `{source_kind: container\|project, source_ref, project_name?}`: 202 |
| GET | `/imports/{id}` | the state, the error and the plan |
| PATCH | `/imports/{id}/plan` | `{choices?: {container_id: bring\|leave}, project_name?, confirm?}` |
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
`container_import_files`; the columns `services.admin_only` and `service_mounts.link_target`.

## Not done yet

The streamed log of an import (the page looks again every 2 seconds), re-checking a link's target at every
later deploy (it is checked when the import makes it), databases as database services (they map to apps), a
partial import of a Compose project does not edit its file (the watch stops what runs again), the Docker
network guard for the originals' networks, the volume's size and the estimated downtime in the plan's
overview, and the plan's choice of a domain or another port for a port the panel keeps.
