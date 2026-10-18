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

Log lines are cut to 16 KiB, a log request stops reading after about 2 MiB of text, live log events
are paced (about 2 MB/s), and a terminal whose output outruns the browser by 8 MiB is closed. The framework limits streams and sockets
per user and per instance.

## Reporting a vulnerability

Please open a private security advisory on GitHub
(https://github.com/irisvn/rdockpanel/security/advisories) rather than a public issue.
