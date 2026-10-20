# Installing RDockPanel

On a fresh Linux server (Ubuntu or Debian, amd64 or arm64), as root:

```sh
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- --admin-email you@example.com
```

It prints the administrator's password and the panel's address (`http://<server-ip>:3000/admin`).

With a domain for the panel and HTTPS certificates (Let's Encrypt), and generated domains for apps:

```sh
curl -fsSL …/install.sh | sudo sh -s -- \
  --domain panel.example.com --email you@example.com \
  --apps-domain apps.example.com --admin-email you@example.com
```

Point `panel.example.com` and `*.apps.example.com` (a wildcard record) to the server first.

| Option | |
|---|---|
| `--domain HOST` | the panel at `https://HOST` only (needs `--email`; point its DNS here first) |
| `--email ADDRESS` | the Let's Encrypt account: certificates for the panel and every HTTPS app domain |
| `--apps-domain HOST` | generated app domains `<project>-<service>.HOST` |
| `--no-domain` | back to `http://<server-ip>:3000` |
| `--admin-email EMAIL` | creates the first administrator and prints its password |
| `--image IMAGE` | the panel's image (default `ghcr.io/irisvn/rdockpanel:main` until the first release) |
| `--version X.Y.Z` | that release of the panel: `ghcr.io/irisvn/rdockpanel:X.Y.Z` |
| `--upgrade-system` | MySQL and Traefik pulled again at their tags (a newer patch of `mysql:8.4`, `traefik:v3.7`) |

The options are kept in `/etc/rdockpanel/install.conf`: a later run changes only what it is given.
That file is read as data (`KEY=value` lines of `IMAGE`, `DOMAIN`, `EMAIL`, `APPS_DOMAIN`, each
value checked), never run. In the panel, **Settings → Panel** checks a new domain, email or apps'
domain (DNS records, a name a service already uses) and gives the exact command to run here.
Without a domain the panel answers on plain HTTP (port 3000), like EasyPanel before its domain is
set: give it a domain before using it over the internet. With one, port 3000 is closed and the
panel is reached over HTTPS only.

## What it sets up

- Docker (when missing) and a single-node **Swarm**.
- `/etc/rdockpanel`: `secrets/` (`APP_KEY`, the database's passwords; made once, kept across
  installs), `traefik/dynamic/` (the routes the panel writes), `traefik/acme/` (certificates).
- Docker secrets `rdp_app_key`, `rdp_database_url`, `rdp_mysql_root`, `rdp_mysql_password`, and the
  overlay network `rdp-internal`.
- Three Swarm services, labelled `rdockpanel.system=true` (hidden from users without
  `system.manage`), pinned to this node (`node.labels.rdockpanel.main`: their data lives here):
  - `rdp-mysql`: MySQL 8.4, data on the volume `rdp-mysql-data`, reachable on `rdp-internal` only;
  - `rdp-traefik`: Traefik v3 on ports 80 and 443;
  - `rdockpanel`: the panel (port 3000 without a domain), with the Docker socket, the host's `/proc` (server
    metrics), the routes directory and `/var/lib/rdockpanel/backups` (the same path inside: what a
    "Local directory" storage provider under it writes lands on the host). Its entrypoint migrates
    the database at each start. It reaches itself at `http://rdockpanel:3000` from Traefik
    (maintenance pages).

## Upgrading and changing options

Run the installer again: it updates the panel to its image (the newest of its tag) and the options
given, keeping the secrets, the database and the certificates. MySQL and Traefik are left running
as they are while their configuration is unchanged (no newer patch of their tag behind the
database's back, no restart of every domain): `--upgrade-system` pulls them again; a changed
`--email` restarts Traefik with its new arguments (the certificate resolvers added in the panel are
kept).

Images: `ghcr.io/irisvn/rdockpanel:main` (the newest commit of `main`), `:main-<commit>` (that
commit), and for a release `vX.Y.Z` the tags `:X.Y.Z`, `:X.Y` and `:latest`. The panel says which
it runs (its version, commit and RustCMS Core's) at the foot of the Dashboard and in
**Settings → Version**. A release's panel checks every 6 hours for a newer release (the
registry's `X.Y.Z` tags; the notes are GitHub's release), says so there and on the Dashboard (to
its administrators) and gives the installer command for it (`--version X.Y.Z`): the panel never
updates itself. A `main` image is a development build: not compared. Settings → Version also
removes the panel's old versions (old tasks' stopped containers and their images; the running and
previous versions kept). A secret file in
`/etc/rdockpanel/secrets` that no longer matches its Docker secret stops the installer: restore
the file from a backup (changing `APP_KEY` needs `rustcms key:reencrypt`; see
[security](security.md)).

## Backups

Your apps' databases: a database service's Backups tab (scheduled dumps to a storage provider of
Settings: a directory under `/var/lib/rdockpanel/backups`, or an S3-compatible bucket, better off
the server). The panel itself: back up `/etc/rdockpanel` (secrets and certificates) and the database: `docker exec` into the
`rdp-mysql` container and `mysqldump panel`. Losing `APP_KEY` loses the encrypted environment
variables, registry passwords and file mounts (see [security](security.md)).

## Uninstalling

`docker service rm rdockpanel rdp-traefik rdp-mysql`, then (this deletes the panel's data)
`docker volume rm rdp-mysql-data`, `docker secret rm rdp_app_key rdp_database_url rdp_mysql_root
rdp_mysql_password`, `docker network rm rdp-internal` and `rm -r /etc/rdockpanel`. Your apps'
services keep running until removed.
