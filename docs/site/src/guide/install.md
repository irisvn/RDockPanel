# Install on Linux

Install RDockPanel on a fresh Ubuntu or Debian server with one command.

## Before you start

- A fresh **Ubuntu or Debian** server, **amd64 or arm64**, with root access (`sudo`).
- Ports **80** and **443** open in your provider's firewall (and **3000** while the panel has no
  domain).
- For HTTPS: a domain for the panel (say `panel.example.com`) with an A record pointing to the
  server, and optionally a wildcard record `*.apps.example.com` for apps' generated domains. Set
  them up **before** you run the installer.

Docker does not have to be installed: the installer adds it when it is missing.

## Install

The simplest install, reached by the server's address:

```sh
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- --admin-email you@example.com
```

With a domain and HTTPS for the panel (Let's Encrypt), and generated domains for apps:

```sh
curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh \
  | sudo sh -s -- \
    --domain panel.example.com --email you@example.com \
    --apps-domain apps.example.com --admin-email you@example.com
```

At the end it prints the **administrator's password** and the **panel's address**. Keep the
password somewhere safe.

::: tip Pick a release
Use `--version X.Y.Z` (the latest is on the
[releases page](https://github.com/irisvn/RDockPanel/releases)). Without it you get the `main`
image, a development build that **Settings → Version** cannot update with a button.
:::

## The panel's database: MySQL or SQLite

The panel keeps its own data (projects, services, users, the audit log…) in one of two databases,
chosen at the first install and kept afterwards:

| | MySQL 8.4 (default) | SQLite |
|---|---|---|
| What runs | a service `rdp-mysql` next to the panel | one file, no database server |
| Memory | more | less |
| Best for | most servers | a small server; one writer at a time, so many people saving at once wait their turn |

Run in a terminal, the installer asks (Enter means MySQL). `--database mysql` or
`--database sqlite` answers beforehand; from a script it is MySQL. This choice does not change
your apps' own databases. It cannot be changed later: a run with another `--database` stops
without changing anything.

## Options

| Option | What it does |
|---|---|
| `--admin-email EMAIL` | creates the first administrator and prints its password |
| `--domain HOST` | the panel at `https://HOST` only (needs `--email`; point its DNS here first) |
| `--email ADDRESS` | the Let's Encrypt account, for the panel's and apps' certificates |
| `--apps-domain HOST` | generated app domains `<project>-<service>.HOST` |
| `--no-domain` | back to `http://<server-ip>:3000` |
| `--database mysql\|sqlite` | the panel's database, at the first install only |
| `--version X.Y.Z` | that release of the panel |
| `--image IMAGE` | another panel image (default: the `main` development build) |
| `--upgrade-system` | pull MySQL and Traefik again at their tags (a newer patch) |
| `--remove-waf` | remove the [WAF](/server/waf) and everything it made, then stop (turn it off first) |

The options are kept in `/etc/rdockpanel/install.conf`. A later run changes only what you give it.

## What it sets up

- Docker (when missing) and a single-node **Swarm**.
- Three Swarm services, hidden from users without the `system.manage` permission:
  - `rdockpanel`: the panel itself;
  - `rdp-traefik`: Traefik v3.7 on ports 80 and 443, which routes every domain;
  - `rdp-mysql`: the panel's MySQL (not with SQLite).
- `/etc/rdockpanel`: the panel's secrets (including `APP_KEY`), Traefik's routes and certificates.
- `/var/lib/rdockpanel`: backups, Compose files, and with SQLite the database (`data/`).

## First sign-in

1. Open the address the installer printed:
   - without a domain: `http://203.0.113.10:3000/admin` (your server's address);
   - with a domain: `https://panel.example.com/admin`.
2. Sign in with the email you gave in `--admin-email` and the printed password.
3. You land on the **Dashboard**. Next: [your first app](/guide/first-app).

The panel's address itself (`/`) shows a public homepage about RDockPanel, with nothing about your
server. **Settings → Panel** turns it off; the panel is always at `/admin`.

::: warning Give it a domain
Without a domain the panel answers on plain HTTP, port 3000. Give it a domain before using it over
the internet: with one, port 3000 is closed and the panel is reached over HTTPS only. **Settings →
Panel** checks a new domain (DNS records, names in use) and shows the exact installer command to
run.
:::

## Change options later

Run the installer again with the options to change. It keeps the secrets, the database and the
certificates. See [Update and back up the panel](/guide/update).

## Uninstall

1. If the [Firewall](/server/firewall) is on, turn it off first (**Firewall → Turn off**).
2. Remove the panel's services (`rdp-mysql` with MySQL; `rdp-cloudflared` and every
   `rdp-cloudflared-*` if you used Cloudflare Tunnel):

   ```sh
   docker service rm rdockpanel rdp-traefik rdp-mysql rdp-cloudflared
   ```

3. Remove its data. **This deletes the panel's database and its backups:**

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
Your apps' services keep running until you remove them. Keep `/var/lib/rdockpanel/compose` until
the Compose services and their data are gone: their `./data` folders live there.
