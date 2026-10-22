# Security

## The Docker socket is root

The panel talks to the Docker daemon through `/var/run/docker.sock`. Whoever controls Docker
controls the host. Treat panel accounts with `docker.manage` or `docker.exec` like root access:

- give people the least they need (see [permissions](permissions.md));
- keep the panel behind HTTPS (Phase 2 installer) and strong passwords; two-factor authentication
  comes with RustCMS TOTP (planned);
- review the audit log (System → Audit log) and the actions log.

## Protected objects

Objects labelled `rdockpanel.system=true` (the panel, its database, its proxy) are hidden from and
untouchable by users without `system.manage`. A container name or id is first checked (letters,
digits, `_`, `.`, `-`, starting with a letter or digit: no `/` or `%`), resolved through Docker's
inspect, and every action then uses the full id, so a crafted name or a rename cannot redirect it to a
protected object. Images are removed only by an id or a tag exactly as Docker lists it. `inspect` hides env values whose name looks secret
(`PASSWORD`, `TOKEN`, `KEY`, `SECRET`, `AUTH`, …) and the password in URLs
(`DATABASE_URL=mysql://user:******@host`).

## Bind mounts

Off by default: `[panel] bind_roots` is empty, and the installer sets none and mounts none. To
allow one, add it to the panel's service yourself, at the same path inside (read-only is enough):
`docker service update --env-add 'RUSTCMS_PANEL__BIND_ROOTS=["/srv/data"]' --mount-add
type=bind,source=/srv/data,target=/srv/data,readonly rdockpanel` (a rerun of the installer keeps
both). A host directory must sit under one of those roots, is resolved with its
symlinks when saved, and is checked again before each deployment. Docker resolves the path once more
when the task starts, so **only list roots that no service and no untrusted user can write**: someone
who can replace a directory there with a symlink could point a later start elsewhere.

## Deployments and routes

- Deploying needs `services.deploy`, destroying `services.destroy`; a deployment's log, error and
  image need `deployments.view` (also on the service page).
- Environment variables and the registry password are sealed with `APP_KEY` (RustCMS encrypted
  columns): the panel shows their names (and whether a password is set); environment values only to
  users with `services.secrets` (the Environment tab), each read audited; the registry password
  never. The audit log, the deployments' configuration and their logs never hold them. Losing `APP_KEY` loses them (set them again); rotate it with `APP_PREVIOUS_KEYS` and
  `rustcms key:reencrypt`.
- The panel never logs them. The Docker client library logs request bodies (a service's spec) at
  debug level through the `log` crate: the panel caps `log` records at info at boot, whatever
  `RUST_LOG` asks.
- Docker holds the values in clear: in the Swarm service's spec and in the containers' environment.
  Anyone who controls Docker on the host (or with `docker.view`, a container's inspect, which hides
  only secret-looking names) can read them.
- File mounts' contents are sealed the same way (`service_mounts.sealed_content`,
  `deployments.files`), written and replaced, never shown back.
- A secret written into a service's command or health check is stored as typed: use environment
  variables instead.
- Services run on their project's own overlay network: a project cannot call another project's
  services directly, nor the panel's MySQL. What a service makes public (a domain, a published
  port) stays reachable by anyone, other projects included. Traefik joins every project's network to
  route to them.
- Hostnames and paths are checked before they become Traefik rules (no backticks or other rule
  syntax), and route files are named after checked service names only. The panel owns
  `[panel] traefik_dir`: at boot it removes route files of services it does not have.
- Docker objects of a service are labelled `rdockpanel.managed=true` with their project and service;
  the panel refuses to touch a service of that name it did not create.

## Templates

Templates are code (TypeScript generators) from a third-party repository (`easypanel-io/templates`
on GitHub, fetched at runtime: it has no license, so it is not bundled). The panel runs
them in a child process of the panel (CPU and memory limits, killed after 15 s), in an embedded
QuickJS with no file system, network or process access, nothing importable but a few random/hash
helpers (16 bcrypt hashes at most), and a memory (64 MB), stack and time (3 s) budget; sources and
fetched files are size-limited. Their output is data, checked like any service the panel creates,
and the related records it asks for (domains, ports, mounts) need their own permissions. A
template needing authentication or capabilities the panel cannot give yet is never deployed at
once. Their images and what they configure are as trusted as
the template: read a template's page before using it.

## GitHub webhooks

A webhook's secret is sealed and shown once; each delivery is checked against its HMAC-SHA256
signature in constant time, and an unsigned or wrongly signed one is answered as if the service did
not exist. Only a push to the service's own branch deploys.

## Builds

A build runs a Dockerfile (the repository's, or one written in the panel) on this node's Docker: it
is as trusted as
the repository. Clone URLs are https or ssh only (no local paths, `file://` or `ext::`), branches
and paths are checked (no options, nothing outside the repository, links resolved: a link in the
repository cannot point the build elsewhere), Git never prompts, and the deploy key is written `0600`
beside the checkout (never in a build context), in a temporary directory removed after the build.
The repository's `.dockerignore` applies (a tracked `.env` it lists stays out of the image). Each clone is limited to 10 minutes, the context to 1 GB. A trigger URL's token is in its query (the
request log records paths only) and stored as its SHA-256 only; anyone with the URL can deploy the service (not change it): make a new one when it
leaks.

## Traefik's files

Traefik reads its dynamic files as Go templates before parsing them: every value the panel writes
there is checked (hosts, paths and names by their character sets; redirects may not contain `{{`
or `}}`), so a user's value never runs a template (which could read Traefik's environment).
Basic authentication hashes are written there but never kept in a deployment's snapshot.

## Rate limits and the visitor's address

A rate limit counts visitors by an address nobody can choose: the peer, or the `X-Forwarded-For` of
a proxy Traefik trusts (Cloudflare's edges on 80/443, cloudflared's network on the tunnel's
entrypoint). Traefik drops that header from anyone else, a project's container included. A service
then sees its Cloudflare visitors' address first in `X-Forwarded-For` (`X-Real-Ip` stays the peer's).
Exempt addresses match the peer only. A Cloudflare Worker fetching a site names its own visitor;
Cloudflare's cache answers without reaching the server; a published port bypasses Traefik.

## Maintenance pages

A maintenance page may be HTML written by a member of the service's project. It is served in a
sandbox (its CSP: no script, no form, an opaque origin) and only for the service's own domains,
never the panel's domain or an address: it cannot act on the panel's origin. The panel's preview of
it is the same response, framed by the panel, for its author only. Through a cache (Cloudflare's), a
page cached before maintenance may still be shown, and a page of a visitor let through may be cached
for others: purge the cache when maintenance starts or ends.

## Compose services

A Compose file can run privileged containers and mount any host directory: creating or editing a
Compose service needs `system.manage`, and so does its environment (the file reads it with
`${VAR}`, even in its volumes). Its `.env` holds its environment in clear (`0600`, in the
Compose directory, root's).

## Backups

A backup holds the database's data in clear (gzipped, not encrypted): keep the bucket private, or
the local directory (`0700` by the installer) to root. The S3 secret key is sealed with `APP_KEY`
and never shown again. Dumps run in the database's own container with its own credentials; the
database name is an argument, never part of the shell script. Provider paths are checked (no `..`,
no absolute path) and a local provider's directory must be absolute and not `/`; under it no link
is followed (`openat` with `O_NOFOLLOW`), so a link planted in the root never leads elsewhere. It is
written as the panel's user, so `storage_providers.*` is an administrator's permission. Dumps are
staged in a private (`0700`) scratch directory; S3 errors never include the signed URLs. A restore replaces
data in place: `backups.restore` is separate from `backups.run`.

## Notifications

A channel's URL, bot token or SMTP password is sealed and never shown again; errors never include
the URL. The panel posts to the URL given (a webhook may point at a private address:
`notification_channels.*` is an administrator's permission) without following redirects.
Notifications carry project, service and database names and errors, never environment values.

## The terminal

- Needs `docker.exec` (the Containers page), or `services.exec` in the service's project (the
  service page, a running task of that service only); `system.manage` for the panel's own
  containers, and for a privileged service's.
- Opened over a WebSocket that requires the panel's exact origin and the session's CSRF token; the
  session is rechecked every 10 s and the terminal stops within 15 s of a revoked access.
- Shells are ended when the session ends: hang-up, then kill, of every process in the shell's session
  (including background jobs and processes that cleared their environment). A shell left by a panel
  crash is ended at the next boot.
- Each session is recorded as an action (`container.exec`) and in the audit log, with its duration.
  Keystrokes and output are never recorded.
- Closes after 30 minutes without input or output.

## Content Security Policy

The admin panel's CSP forbids inline scripts and styles (`script-src 'self'; style-src 'self'`). The
pages build the DOM with elements and text nodes, never `innerHTML` with Docker data. xterm.js injects
`<style>` elements, which the CSP blocks: `resources/admin/js/rdockpanel/vendor/csp-shim.js` turns the
`<style>` elements that scripts create into constructed stylesheets instead. Parsed markup is
untouched, so injected HTML still cannot add styles. The policy itself is unchanged.

The homepage at `/` is public: one static page (English or Vietnamese), its stylesheets, images,
favicon and one same-origin script (`/home/home.js`, the screens' viewer: a dialog with
previous/next, arrow keys, swipe, Esc), all built into the panel. It has a policy of its own,
stricter still (`default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self';
base-uri 'none'; form-action 'none'; frame-ancestors 'none'`, no inline script), and nothing of the
installation: no version, address, domain or project (its screenshots are of a demo server). Only
the brand color of Settings → Branding comes from the database. Settings → Panel turns it off
(`system.manage`, audited as `home.update`): `/` then redirects to `/admin`.

## The firewall

The Firewall page ([firewall.md](firewall.md)) changes the host's nftables, so it is `system.manage`
only and installed panels only. It never takes raw rules: ports, protocols and canonical CIDRs are
checked, notes are never rendered, and the host scripts are fixed texts in the binary (the table on
standard input, checked tokens as arguments). They run from a short-lived privileged helper
container in the host's namespaces, as Reboot does. Against locking yourself out: SSH and 80/443
are always open and are accepted before the blocked addresses (SSH) or the drops; each change is
reverted by the host itself after 120 s unless kept; a boot that cannot load the confirmed table
loads the fallback (SSH and the web only), and Docker does not start unfiltered.

## Live data limits

Log lines are cut to 16 KiB. A log request (the last lines) stops reading after about 2 MiB of text;
following logs live has no total limit, but its events are paced (about 2 MB/s). The framework queues at most 1 MiB (64 events) per stream and closes one
whose browser falls behind (`lagging`); a socket send that takes over 10 s ends it. It also limits
streams and sockets per user and per instance.

## Reporting a vulnerability

Please open a private security advisory on GitHub
(https://github.com/irisvn/rdockpanel/security/advisories) rather than a public issue.

## Client addresses and sign-in alerts

The panel reads a client's address from `X-Forwarded-For` only when the connection comes from a
trusted proxy (`http.trusted_proxies`). The installer trusts the private ranges Traefik reaches the
panel from (`10.0.0.0/8`, `172.16.0.0/12`); a client connecting to port 3000 directly is never
trusted. Without that setting, every sign-in seems to come from Traefik, and the "sign-in from a new
address" notification (off by default) says nothing useful. Running the panel behind another proxy
(Cloudflare, a load balancer), add its addresses there.

Traefik drops request headers whose names alias others (`X_Forwarded_For` beside
`X-Forwarded-For`: `aliasHeadersStrategy=delete` on every entrypoint), so an app whose runtime reads
both alike (PHP, WSGI) never takes a client's forged one for Traefik's.

## Version checks

Settings → Version asks, every 6 hours (by hand at most once a minute), the registry
(`ghcr.io/irisvn/rdockpanel`, anonymously, as `docker pull` does) for its `X.Y.Z` tags and GitHub
for the release notes; nothing about the server is sent (a user agent with the panel's version).
The panel trusts the registry: images are not signed. It never updates by itself: an
administrator presses Update (`system.manage`, audited as `system.panel.update`). The image is
always `ghcr.io/irisvn/rdockpanel:<version>`, pinned to the digest the panel asked the registry
for (never one the browser sends); the panel's database is dumped first (readable by root only,
in the backups directory), and Swarm rolls back to the previous image if the new one does not
become healthy. The rolled-back version starts on the database as the new one left it (`migrate
--allow-newer`, RustCMS 0.8.1: from 0.5.0 on; the panel's migrations only add — new tables,
nullable or defaulted columns); the dump is there if anything else is needed
([install](install.md#restoring-the-panels-database)). Release notes are shown as text, never as HTML.

## GitHub tokens

A token of Settings → Git is sealed with `APP_KEY`, never shown again, and usable only by the
projects it is given to: their developers (`services.update`) can list and clone every repository
it reaches, so give each project the narrowest token (a fine-grained one, read-only, for the
repositories it needs). Git gets it through `GIT_ASKPASS` from a file of its own (mode 0600),
removed once the clone ends: never in a URL, an argument, `.git/config` or a log (the deployment's
log hides it).
