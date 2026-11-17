# Security

What RDockPanel does to keep your server safe, and what is left to you.

::: warning The panel controls Docker, and Docker is root
The panel talks to Docker through its socket, and whoever controls Docker controls the server.
Treat an account that may manage containers (`docker.manage`) or open a terminal (`docker.exec`)
like root access.
:::

## What the panel does

**Access is denied by default.** A user can do only what their roles grant, and a project member
only what their project role grants, in their projects. Every page and route checks its
permission before reading the request; live logs and terminals check again every few seconds and
stop soon after a permission is lost or the user signs out. See [Users and roles](/server/users).

**The panel's own objects are protected.** The panel, its database and Traefik are labelled
`rdockpanel.system=true`. Without the `system.manage` permission they are hidden from every list
and cannot be inspected, changed or pruned.

**Administrator-only actions** need `system.manage`: the Firewall, Settings' Panel, Version,
Traefik, Certificates, Cloudflare Tunnel, Branding, Maintenance page and Git tabs; Compose
services; host directories; added capabilities, sysctls and groups; wildcard domains and
certificate choices; importing containers; choosing where a service runs.

**Secrets are encrypted.** Environment variables, registry passwords, file mounts' contents, the
private half of deploy keys, webhook secrets and Cloudflare Tunnel tokens are sealed with the panel's key (`APP_KEY`). Values are shown only to users
with `services.secrets`, and each read is written to the audit log first. The registry password is
never shown back. A container's details hide values whose name looks secret, passwords in URLs and
on command lines.

**Everything is recorded.** Changes to the panel's data go to the audit log (**System → Audit
log**), Docker operations to **Actions**, with who did them; deployments keep their own history and
log.

**Projects are apart.** Each project has its own network: a project's services cannot call another
project's services, nor the panel's database. What a service makes public (a domain, a published
port) is reachable by anyone.

**A default-deny [firewall](/server/firewall)** (Linux servers) closes every port except SSH, 80
and 443 and those you open; each change is undone after two minutes unless you keep it.

**Tokens and URLs** are shown once and stored as a hash where they only need checking (deployment
trigger URLs); a GitHub webhook's signature is checked on every delivery. API tokens expire within
a year and never exceed their scopes.

## What to do yourself

- **Give the panel a domain** so it runs on HTTPS only (port 3000 closes).
- **Use strong, unique passwords.** Two-factor authentication is not available yet.
- **Grant the least** each person needs. Prefer project members over global roles.
- **Update** the panel when a new release is out ([Update](/guide/update)), and the server's own
  packages.
- **Back up** `/etc/rdockpanel` (it holds `APP_KEY`) and the panel's database, off the server.
  Losing `APP_KEY` loses every encrypted value.
- **Keep secrets in environment variables**, not in a service's command or health check (those
  are stored as typed).
- **Read templates and repositories before deploying them**: their images and builds are as
  trusted as their authors.
- **Review** the audit log and Actions from time to time.

The developer details are in
[docs/security.md](https://github.com/irisvn/RDockPanel/blob/main/docs/security.md).
