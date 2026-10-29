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

### The panel's database

The panel keeps its own data (projects, services, users, the audit log, …) in one of two
databases, chosen at the first install and kept afterwards:

- **MySQL 8.4** (the default): a service `rdp-mysql` next to the panel. For most servers.
- **SQLite**: one file, `/var/lib/rdockpanel/data/panel.sqlite`, no database server (less memory).
  For a small server: one writer at a time, so many people saving at once wait their turn.

Run in a terminal, the installer asks (Enter is MySQL); `--database mysql` or `--database sqlite`
answers it beforehand, and without a terminal (a script, cron) it is MySQL. Your apps' own
databases (PostgreSQL, MySQL, … services) are not affected by this choice.

The choice is kept in `install.conf` (`DATABASE=`). A later run with another `--database` stops
without changing anything: moving the panel's data from one database to the other is not done by
the installer. An `install.conf` from before 0.10.0 has no `DATABASE`: the installer then reads it
from what the install left (`rdp-mysql`, the database URL secret), and stops when it cannot tell
(put `DATABASE=mysql` or `DATABASE=sqlite` in the file).

| Option | |
|---|---|
| `--domain HOST` | the panel at `https://HOST` only (needs `--email`; point its DNS here first) |
| `--email ADDRESS` | the Let's Encrypt account: certificates for the panel and every HTTPS app domain |
| `--apps-domain HOST` | generated app domains `<project>-<service>.HOST` |
| `--no-domain` | back to `http://<server-ip>:3000` |
| `--admin-email EMAIL` | creates the first administrator and prints its password |
| `--database mysql\|sqlite` | the panel's database, at the first install only (see above; asked in a terminal, else MySQL) |
| `--image IMAGE` | the panel's image (default `ghcr.io/irisvn/rdockpanel:main`, a development build: use `--version` for a release) |
| `--version X.Y.Z` | that release of the panel: `ghcr.io/irisvn/rdockpanel:X.Y.Z` |
| `--upgrade-system` | MySQL and Traefik pulled again at their tags (a newer patch of `mysql:8.4`, `traefik:v3.7`) |

The options are kept in `/etc/rdockpanel/install.conf`: a later run changes only what it is given.
That file is read as data (`KEY=value` lines of `IMAGE`, `DOMAIN`, `EMAIL`, `APPS_DOMAIN`, `DATABASE`, each
value checked), never run. In the panel, **Settings → Panel** checks a new domain, email or apps'
domain (DNS records, a name a service already uses) and gives the exact command to run here.
Without a domain the panel answers on plain HTTP (port 3000): give it a domain before using it
over the internet. With one, port 3000 is closed and the
panel is reached over HTTPS only.

## What it sets up

- Docker (when missing) and a single-node **Swarm**.
- `/etc/rdockpanel`: `secrets/` (`APP_KEY`, the database's URL and MySQL's passwords; made once,
  kept across installs), `traefik/dynamic/` (the routes the panel writes), `traefik/acme/`
  (certificates).
- Docker secrets `rdp_app_key`, `rdp_database_url` (and with MySQL `rdp_mysql_root`,
  `rdp_mysql_password`), and the overlay network `rdp-internal`.
- With SQLite, `/var/lib/rdockpanel/data` (the database file and its `-wal`/`-shm` companions),
  mounted into the panel at the same path.
- Three Swarm services (two with SQLite: no `rdp-mysql`), labelled `rdockpanel.system=true` (hidden from users without
  `system.manage`), pinned to this node (`node.labels.rdockpanel.main`: their data lives here):
  - `rdp-mysql`: MySQL 8.4, data on the volume `rdp-mysql-data`, reachable on `rdp-internal` only;
    under 2 GB of memory it runs small (no `performance_schema`, no binary log, smaller buffers
    and caches: the panel's database is some 50 tables), and it gets 120 seconds to shut down;
  - `rdp-traefik`: Traefik v3.7 on ports 80 and 443, and an entrypoint `tunnel` (`:8081`, never
    published) that Cloudflare Tunnel's routes use while the tunnel is on;
  - `rdockpanel`: the panel (port 3000 without a domain), with the Docker socket, the host's `/proc` (server
    metrics), the routes directory, `/var/lib/rdockpanel/backups` (the same path inside: what a
    "Local directory" storage provider under it writes lands on the host) and
    `/var/lib/rdockpanel/compose` (Compose services' files, the same path inside), and with SQLite
    `/var/lib/rdockpanel/data`. It is replaced stop-first (never two panels writing). Its entrypoint migrates
    the database at each start. It reaches itself at `http://rdockpanel:3000` from Traefik
    (maintenance pages).

The panel's address (`/`) shows a public homepage: what RDockPanel is, its screens, the install
command and a link to GitHub, with nothing of this server on it. **Settings → Panel** turns it off
(`/` then opens the sign-in). The panel itself is at `/admin`.

Settings adds more as they are used: Cloudflare Tunnel's service `rdp-cloudflared`, its network
`rdp-tunnel` and its token as secrets `rdp_tunnel_*` (labelled `rdockpanel.system=true` too); DNS
resolvers' tokens as secrets `rdp-dns-*` (Traefik reads them).

## Upgrading and changing options

Run the installer again: it updates the panel to its image (the newest of its tag) and the options
given, keeping the secrets, the database and the certificates. MySQL and Traefik are left running
as they are while their configuration is unchanged (no newer patch of their tag behind the
database's back, no restart of every domain): `--upgrade-system` pulls them again; a changed
`--email` restarts Traefik with its new arguments, and MySQL restarts once when its settings
change (a first rerun after this release, or a server with more or less memory) (the certificate resolvers added in the panel are
kept).

Images: `ghcr.io/irisvn/rdockpanel:main` (the newest commit of `main`), `:main-<commit>` (that
commit), and for a release `vX.Y.Z` the tags `:X.Y.Z`, `:X.Y` and `:latest`. The panel says which
it runs (its version, commit and RustCMS Core's) at the foot of the Dashboard and in
**Settings → Version**. A release's panel checks every 6 hours for a newer release (the
registry's `X.Y.Z` tags; the notes are GitHub's release), says so there and on the Dashboard (to
its administrators). Its **Update** button (an administrator's; the panel never updates by itself)
moves the panel to that release: new deployments, backups and restores wait while those running
end, the panel's database is copied to `/var/lib/rdockpanel/backups/panel/` (a `mysqldump`, or with
SQLite a checked `VACUUM INTO` copy; no copy, no update), then Swarm replaces
the panel (the release's image pinned by its digest) and rolls back if the new one does not
become healthy; the next panel records how it ended (Settings → Version → History). The
installer command for it (`--version X.Y.Z`) is shown too. A run of the installer without
`--version` or `--image` keeps a newer release the panel updated to. A `main` image is a
development build: not compared, no button. Settings → Version also
removes the panel's old versions (old tasks' stopped containers and their images; the running and
previous versions kept). A secret file in
`/etc/rdockpanel/secrets` that no longer matches its Docker secret stops the installer: restore
the file from a backup (changing `APP_KEY` needs `rustcms key:reencrypt`; see
[security](security.md)).

### Restoring the panel's database

Settings → Version's Update dumps the panel's database first, to
`/var/lib/rdockpanel/backups/panel/rdockpanel-<version>-<time>-<id>.sql`. When the new version
fails, Swarm rolls back to the previous one, which starts on the database as the new one left it
(from 0.5.0 on: `migrate --allow-newer`; Settings → Version then lists the newer migrations).
Rolling back to a version before 0.5.0, or undoing what a release changed, needs the dump:

```sh
c=$(docker ps -q -f name=rdp-mysql)
docker exec -i "$c" sh -c 'MYSQL_PWD="$(cat /run/secrets/rdp_mysql_root)" mysql -uroot' \
  < /var/lib/rdockpanel/backups/panel/rdockpanel-0.5.0-….sql
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- --version 0.5.0
```

With SQLite the copy is `rdockpanel-<version>-<time>-<id>.sqlite`, a whole database. Install the
version the copy was made by, stop the panel and wait until its task is gone (`docker ps` shows no
`rdockpanel` container), replace the file and remove the old `-wal` and `-shm` files (they belong
to the old file), then start it:

```sh
curl -fsSL …/install.sh | sudo sh -s -- --version 0.10.0
docker service scale rdockpanel=0
cd /var/lib/rdockpanel/data
rm -f panel.sqlite-wal panel.sqlite-shm
cp /var/lib/rdockpanel/backups/panel/rdockpanel-0.10.0-….sqlite panel.sqlite
docker service scale rdockpanel=1
```

## Backups

Your apps' databases: a database service's Backups tab (scheduled dumps to a storage provider of
Settings: a directory under `/var/lib/rdockpanel/backups`, or an S3-compatible bucket, better off
the server). The panel itself: back up `/etc/rdockpanel` (secrets and certificates) and the database: with
MySQL, `docker exec` into the `rdp-mysql` container and `mysqldump panel`; with SQLite, copy the
whole `/var/lib/rdockpanel/data` directory while the panel is stopped (`docker service scale
rdockpanel=0`, then `=1`): the latest changes may still be in `panel.sqlite-wal`, so
`panel.sqlite` alone is not a backup. Or keep the copy Settings → Version's Update makes (one
self-contained file). Losing `APP_KEY` loses the encrypted environment
variables, registry passwords and file mounts (see [security](security.md)).

## Uninstalling

If the Firewall is on, turn it off first (Firewall → Turn off): Docker waits for it at boot. Without
the panel, `sh /var/lib/rdockpanel/firewall/fw.sh disable` does the same.

Then `docker service rm rdockpanel rdp-traefik rdp-mysql rdp-cloudflared` (`rdp-mysql` with MySQL,
the last one if the tunnel was on), then (this deletes the panel's data, its backups included):

```sh
docker volume rm rdp-mysql-data                                         # MySQL
docker secret rm rdp_app_key rdp_database_url
docker secret rm rdp_mysql_root rdp_mysql_password                      # MySQL
docker secret ls -q -f name=rdp_tunnel_ -f name=rdp-dns- | xargs -r docker secret rm
docker network rm rdp-internal rdp-tunnel     # rdp-tunnel only if the tunnel was used
rm -r /etc/rdockpanel /var/lib/rdockpanel/backups
rm -r /var/lib/rdockpanel/data                                          # SQLite
```

Remove the database too: a new install makes a new `APP_KEY`, which cannot read an old database.

Your apps' services keep running until removed. `/var/lib/rdockpanel/compose` holds the Compose
services' files, and their relative bind mounts (`./data`) live there too: remove it only once
those services and their data are gone.
