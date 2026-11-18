# Features

Everything RDockPanel does today. [Tiếng Việt](features.vi.md) · back to the [README](../README.md).

**The interface**

- `/admin` opens the **Dashboard**: server tiles, projects with their service cards, **+ Service**
  and **Templates**. The sidebar has Dashboard, Domains, Firewall, Actions, Settings and the Docker pages.
- A **service page** has its header (status, Deploy, Start/Stop, Restart, Open, Logs, Shell) and tabs:
  Overview, Credentials (databases), Backups (databases and services with volumes), Deployments,
  Environment, Domains, Redirects, Security, Maintenance, Scripts, Source, Storage, Advanced.
- **Actions** lists deployments, backups and Docker actions; **Settings** has the tabs Panel,
  Version, Traefik, Certificates, Cloudflare Tunnel, Branding, Maintenance page, Git, Backups,
  Notifications and API tokens; **⌘K** finds a project or a service. Light by default, with a
  Light / Dark / System switch per user; in English and Vietnamese.
- Every page follows one set of design rules ([ui-guidelines.md](ui-guidelines.md)): the
  same button sizes, extra row actions in a ⋯ menu, the same status words and colors, the panel's own dialogs,
  tables that work on a phone.

**Importing existing containers** ([import.md](import.md))

- A container (or a Compose project, with the containers it needs) started outside the panel becomes
  a service of a project: read, planned (what blocks, what is lost), stopped, copied, deployed, and the
  originals kept until the import is finished, so it can be undone. Folders on the machine are used in
  place and can be moved into volumes later. Traefik labels and `VIRTUAL_HOST` become the service's domains; a cloudflared container can become
  a Cloudflare Tunnel of the panel (one per token); a password on a command line moves to an environment variable; a container that ran as another
  user keeps it. An official PostgreSQL, MySQL, MariaDB or MongoDB container can become a database service
  (connection details, dumps) with its configuration kept; a domain behind a Traefik IP allow list keeps it.
- **Where a service runs** (Advanced tab, administrators): automatic, one node of the Swarm, or any node
  (shared storage).

**Server and Docker**

- **Server metrics** on the Dashboard: live CPU, memory, disk and network in/out, with their recent
  history.
- **Containers**: list, search, start/stop/restart/pause/kill/remove; live logs (follow, stderr
  highlighted), live stats, inspect (secret-looking env values hidden), and a **terminal** in the
  browser.
- **Images**: pull (progress shown live), remove per tag, prune stopped containers and dangling images.
- **Volumes and networks**: create and remove (bridge or overlay).
- **Nodes**: the Swarm's servers; add one (a join command to copy), pause, drain (what it moves, said first), activate, remove.
- **Safety**: the panel's own containers, volumes and networks (labelled `rdockpanel.system=true`) are
  hidden and untouchable without the `system.manage` permission; prune skips them.
- **Accountability**: every Docker management operation (containers, images, volumes, networks,
  terminals) is recorded in an actions log and the audit log, with who did it; deployments keep their
  own history (who asked, when, the log).
- **Users and roles**: fine-grained permissions (`docker.view`, `docker.manage`, `docker.logs`,
  `docker.exec`, `server.view`, `system.manage`, plus one set per resource such as `services.deploy`),
  built on RustCMS's roles.

**Apps on Docker Swarm**

- **Projects and services**: a service runs a Docker image on **Docker Swarm** with replicas, CPU and
  memory limits, a health check, named volumes, published ports and a command. Each project has its
  own network: a project's services cannot call another project's services directly (what is public,
  through a domain or a published port, stays public).
- **Deployments**: Deploy, Start, Stop, Restart and Destroy from the service page, one at a time per
  service, with a live log; Rollback from an earlier deployment ("Roll back to this"); Force rebuild
  (no build cache, the image pulled again, every task replaced); the Overview's recent errors. Images are
  pinned to their registry digest (a local-only image to its id, with a warning). Updates are rolling
  (start-first when zero downtime is on, the service has a health check and publishes no host-mode
  port; stop-first otherwise) and Swarm rolls back a failed one. Every deployment keeps its
  configuration, for history and rollback, and the page shows changes not deployed yet. Before a
  service is created or deployed, the panel estimates the memory it takes and warns when the server
  runs low (a confirmation when it does not fit; never blocking deploys from the API or webhooks).
- **Domains and HTTPS** through **Traefik**: domains and paths per service; a domain with HTTPS on
  redirects its HTTP to HTTPS; wildcard domains (`*.example.com`, for administrators) and custom
  Traefik middlewares; redirects (regular expressions), a rate limit by visitor (their real address
  through Cloudflare too, with exempt addresses), HTTP basic authentication, and a maintenance page (built in or your own HTML, with when you are
  back, and addresses that still reach the service).
  A **Domains** page lists every service's domains, where they lead and whether they are served.
  A **web application firewall** (the WAF page, CrowdSec's AppSec in Traefik): detect or block,
  basic (known exploits, probes of secret files) or strict (+ the OWASP Core Rule Set), trusted
  addresses, on or off per service; its start followed step by step; requests checked and blocked
  hour by hour, alerts kept 30 days with filters, rules updated from CrowdSec (the bundled ones back if
  that fails), exceptions (a rule spared one domain), an optional blocklist (your bans and CrowdSec's
  community list), Cloudflare's ranges checked daily; it fails open, never taking a site down.
  Blocking an address is shared with the Firewall: one dialog and one list, on the websites (WAF),
  the whole server (Firewall) or both.
  Domains, redirects, rate limits, basic authentication and maintenance **apply when saved** (no Deploy, the
  running containers untouched); image, environment, ports, mounts and resources need a Deploy.
  A certificate resolver per domain (Settings → Certificates: Let's Encrypt over HTTP or over DNS with
  a Cloudflare, DigitalOcean, Hetzner, Vultr or Linode token), applied to Traefik without SSH; or a
  certificate of your own, uploaded (checked, its key sealed, warned of before it expires).
- **Settings → Version**: what the panel runs (and with: Core, Traefik, MySQL or SQLite, Docker, the OS), whether a
  newer release is out (checked every 6 hours, also said on the Dashboard), what changed, an **Update**
  button (an administrator's: the panel never updates by itself; its database dumped first, a rollback
  if the new version does not start), its history, and its old versions removed.
- **Settings → Panel**: the panel's logs and restart; Refresh IP and Reboot
  for the server; Traefik's logs and restart; Docker prunes that never touch the services' data.
- **Firewall** (Workspace menu, administrators): every port closed to the internet except SSH, 80
  and 443, and the ports you open for the addresses you choose (a VPN's, say), for this server or a
  service's published port; blocked addresses; the listening ports and who owns them. Each change is
  tried first and reverted by the server itself unless kept; loaded at boot before Docker
  ([firewall.md](firewall.md)).
- **A homepage** at the panel's address (`/`): what RDockPanel is, its screens, the install command
  and a link to GitHub, in English or Vietnamese, with nothing of this server on it. Settings → Panel
  turns it off (`/` then opens the sign-in).
- **Databases**: PostgreSQL, MySQL, MariaDB, MongoDB and Redis in one click (a name is enough:
  credentials generated, data on a volume, health-checked), with their connection URL to copy.
- **800+ ready-made app templates** in one click (searched, filled in by their own form, deployed with
  their databases), fetched when you open the catalog; or paste a template's output ("Create from
  Schema").
- **Builds**: an app from a Git repository (https, ssh with a generated deploy key, or a GitHub token of
  Settings → Git given to its project, its repositories picked from a list; a branch, tag
  or commit; a sub-directory), built with its Dockerfile, or from a Dockerfile written in the
  panel; the build's log is the deployment's. A deployment trigger URL deploys from a CI, and a
  signed GitHub webhook deploys on each push (auto deploy).
- **Compose services**: a Compose file (written in the panel or from Git) run with `docker compose`
  on the project's network, its domains leading to one of its services.
- **Backups**: scheduled dumps (cron) of PostgreSQL, MySQL, MariaDB and MongoDB, and archives of
  volumes, to a local directory or an S3-compatible bucket, with retention, a log per run, "Run now"
  and Restore.
- **Project members** (viewer, developer, admin per project) and a **public API** with scoped tokens
  (Settings → API tokens) and its command line, `rdp`.
- **Notifications**: Discord, Slack, Telegram, email (SMTP) and webhooks; each channel picks its
  events (deployments, backups, a full disk, a failing service and its return, Traefik down, expiring
  certificates, a newer panel, Docker prunes, a sign-in from a new address), with "Send test notification".
- **Panel in Settings**: its domain, Let's Encrypt email and apps' domain, checked (DNS, names in
  use) with the installer command that applies them.
- **Traefik in Settings**: its state, a restart, its live logs, and middlewares of your own
  (`custom-<name>`, headers, rate limits, IP allow lists…) for domains, all without SSH.
- **Cloudflare Tunnel** (Settings): one cloudflared per tunnel, each run with its own token (kept encrypted,
  given as a Swarm secret; add, switch on or off, delete several tunnels), so domains are reached through Cloudflare without opening ports 80 and 443.
  A domain leading to Cloudflare is marked "via Cloudflare" (through the tunnel its HTTPS is Cloudflare's).
- **Branding** (Settings): the panel's name, primary color, logos (light and dark) and favicon,
  on every page and the sign-in page, without a restart.
- **Roles page**: permissions in areas (Apps, Docker, Server, System) with labels and descriptions,
  a search, and presets (apps viewer, developer, administrator; monitor) to start a role from.
- **Per service**: CPU and memory charts, notes for the team, scripts (one-off commands in the running
  container, their output shown), and, for administrators, capabilities, sysctls and supplemental
  groups.
- **Secrets**: environment variables and private registry credentials, encrypted with the app key;
  edited as `.env` text; values are shown only with the `services.secrets` permission (each read is
  audited). A rollback brings back the environment of then.

**Coming next** (see the [roadmap](roadmap.md))

- Uploads as a source; a web database client.
- Later: moving a service's data to another node, two-factor authentication.
