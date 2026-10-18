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

Off by default. A host directory must sit under one of `[panel] bind_roots`, is resolved with its
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

EasyPanel templates are code (TypeScript generators) from a third-party repository. The panel runs
them in a child process of the panel (CPU and memory limits, killed after 15 s), in an embedded
QuickJS with no file system, network or process access, nothing importable but a few random/hash
helpers (16 bcrypt hashes at most), and a memory (64 MB), stack and time (3 s) budget; sources and
fetched files are size-limited. Their output is data, checked like any service the panel creates,
and the related records it asks for (domains, ports, mounts) need their own permissions. A
template needing authentication or capabilities the panel cannot give yet is never deployed at
once. Their images and what they configure are as trusted as
the template: read a template's page before using it.

## Builds

A build runs the repository's Dockerfile (or Nixpacks') on this node's Docker: it is as trusted as
the repository. Clone URLs are https or ssh only (no local paths, `file://` or `ext::`), branches
and paths are checked (no options, nothing outside the repository, links resolved: a link in the
repository cannot point the build elsewhere), Git never prompts, and the deploy key is written `0600`
beside the checkout (never in a build context), in a temporary directory removed after the build.
The repository's `.dockerignore` applies (a tracked `.env` it lists stays out of the image). Each clone and
Nixpacks run is limited to 10 minutes, the context to 1 GB. A trigger URL's token is in its query (the
request log records paths only) and stored as its SHA-256 only; anyone with the URL can deploy the service (not change it): make a new one when it
leaks.

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

- Needs `docker.exec` (and `system.manage` for the panel's own containers).
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

## Live data limits

Log lines are cut to 16 KiB. A log request (the last lines) stops reading after about 2 MiB of text;
following logs live has no total limit, but its events are paced (about 2 MB/s). The framework queues at most 1 MiB (64 events) per stream and closes one
whose browser falls behind (`lagging`); a socket send that takes over 10 s ends it. It also limits
streams and sockets per user and per instance.

## Reporting a vulnerability

Please open a private security advisory on GitHub
(https://github.com/irisvn/rdockpanel/security/advisories) rather than a public issue.
