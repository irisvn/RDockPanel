#!/bin/sh
# RDockPanel installer, for a fresh Linux server (Ubuntu or Debian):
#
#   curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh | sudo sh
#   curl -fsSL …/install.sh | sudo sh -s -- --domain panel.example.com --email you@example.com \
#       --apps-domain apps.example.com --admin-email you@example.com
#
# Options (kept in /etc/rdockpanel/install.conf: a later run changes only
# what it is given):
#   --domain HOST        the panel at https://HOST only (needs --email; DNS first)
#   --email ADDRESS      Let's Encrypt account: HTTPS certificates for the panel and the apps
#   --apps-domain HOST   generated app domains <project>-<service>.HOST (a wildcard DNS record)
#   --no-domain          back to http://<server-ip>:3000 (forgets --domain)
#   --admin-email EMAIL  creates the first administrator (prints its password)
#   --image IMAGE        the panel's image (default ghcr.io/irisvn/rdockpanel:main)
#
# Run it again any time (to upgrade, or to change the options): what exists
# is kept, its secrets first (APP_KEY, the database's passwords).
set -eu

ETC=/etc/rdockpanel
# Backups to a "Local directory" storage provider (the same path in the panel).
BACKUPS=/var/lib/rdockpanel/backups
# Compose services' files (relative paths of a Compose file resolve on the host).
COMPOSE=/var/lib/rdockpanel/compose
CONF="$ETC/install.conf"
SYSTEM=rdockpanel.system=true
MYSQL_IMAGE=mysql:8.4
TRAEFIK_IMAGE=traefik:v3.5

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf 'rdockpanel: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run it as root (sudo)"
[ "$(uname -s)" = Linux ] || die "RDockPanel installs on Linux"

# What the last run chose, then what this one says.
IMAGE="ghcr.io/irisvn/rdockpanel:main"
DOMAIN=""
EMAIL=""
APPS_DOMAIN=""
# Read as data, never run: KEY=value lines of these keys only (a value with
# anything but the characters its option allows is refused below).
if [ -f "$CONF" ]; then
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      ''|'#'*) continue ;;
      IMAGE=*) IMAGE="${line#IMAGE=}" ;;
      DOMAIN=*) DOMAIN="${line#DOMAIN=}" ;;
      EMAIL=*) EMAIL="${line#EMAIL=}" ;;
      APPS_DOMAIN=*) APPS_DOMAIN="${line#APPS_DOMAIN=}" ;;
      *) die "$CONF: unknown line (keys: IMAGE, DOMAIN, EMAIL, APPS_DOMAIN)" ;;
    esac
  done < "$CONF"
fi
ADMIN_EMAIL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --domain) DOMAIN="${2:-}"; shift 2 ;;
    --email) EMAIL="${2:-}"; shift 2 ;;
    --apps-domain) APPS_DOMAIN="${2:-}"; shift 2 ;;
    --no-domain) DOMAIN=""; shift ;;
    --admin-email) ADMIN_EMAIL="${2:-}"; shift 2 ;;
    --image) IMAGE="${2:-}"; shift 2 ;;
    -h|--help) sed -n '2,19p' "$0" 2>/dev/null || true; exit 0 ;;
    *) die "unknown option: $1" ;;
  esac
done

# Checked before anything changes: these become Docker arguments and
# Traefik rules (no spaces, quotes, backticks).
# One line each (grep checks lines: a value with a newline is refused first).
one_line() { [ "$(printf '%s' "$1" | wc -l)" -eq 0 ]; }
host_ok() { one_line "$1" && printf '%s' "$1" | grep -Eq '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?(\.[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?)+$'; }
email_ok() { one_line "$1" && printf '%s' "$1" | grep -Eq '^[A-Za-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}$'; }
[ -z "$DOMAIN" ] || host_ok "$DOMAIN" || die "--domain: not a host name: $DOMAIN"
[ -z "$APPS_DOMAIN" ] || host_ok "$APPS_DOMAIN" || die "--apps-domain: not a host name: $APPS_DOMAIN"
[ -z "$EMAIL" ] || email_ok "$EMAIL" || die "--email: not an email address: $EMAIL"
[ -z "$ADMIN_EMAIL" ] || email_ok "$ADMIN_EMAIL" || die "--admin-email: not an email address"
one_line "$IMAGE" && printf '%s' "$IMAGE" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9._/:@-]*$' || die "--image: not an image reference"
[ -z "$DOMAIN" ] || [ -n "$EMAIL" ] || die "--domain needs --email (its HTTPS certificate)"

# --- Docker and Swarm -------------------------------------------------------
if ! command -v docker >/dev/null 2>&1; then
  say "Installing Docker"
  curl -fsSL https://get.docker.com | sh
fi
docker info >/dev/null 2>&1 || die "Docker is not running"

ADDRESS=$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{for (i = 1; i < NF; i++) if ($i == "src") print $(i + 1)}' | head -n1)
[ -n "$ADDRESS" ] || ADDRESS=$(hostname -I 2>/dev/null | awk '{print $1}')
[ -n "$ADDRESS" ] || die "could not find this server's address"

# A swarm only when this engine is in none (its state can read "error" while
# it works); it must then be a manager.
if [ "$(docker info --format '{{.Swarm.LocalNodeState}}')" = inactive ]; then
  say "Starting Docker Swarm on $ADDRESS"
  docker swarm init --advertise-addr "$ADDRESS" >/dev/null
fi
docker node ls >/dev/null 2>&1 || die "this server is in a Swarm but not one of its managers"
# The panel's services stay on this node: its database volume, routes and
# certificates live here.
NODE=$(docker info --format '{{.Swarm.NodeID}}')
docker node update --label-add rdockpanel.main=true "$NODE" >/dev/null
PLACE=node.labels.rdockpanel.main==true

# --- Directories, options and secrets (made once, kept) -----------------------
mkdir -p "$ETC/secrets" "$ETC/traefik/dynamic" "$ETC/traefik/acme" "$BACKUPS" "$COMPOSE"
chmod 0700 "$BACKUPS" "$COMPOSE"
chmod 0700 "$ETC" "$ETC/secrets" "$ETC/traefik/acme"
touch "$ETC/traefik/acme/acme.json"
chmod 0600 "$ETC/traefik/acme/acme.json"
umask 077
printf 'IMAGE=%s\nDOMAIN=%s\nEMAIL=%s\nAPPS_DOMAIN=%s\n' "$IMAGE" "$DOMAIN" "$EMAIL" "$APPS_DOMAIN" > "$CONF"

random_hex() { head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n'; }
[ -s "$ETC/secrets/app_key" ] || printf 'base64:%s' "$(head -c 32 /dev/urandom | base64 | tr -d '\n')" > "$ETC/secrets/app_key"
[ -s "$ETC/secrets/mysql_root" ] || random_hex > "$ETC/secrets/mysql_root"
[ -s "$ETC/secrets/mysql_password" ] || random_hex > "$ETC/secrets/mysql_password"
printf 'mysql://panel:%s@rdp-mysql:3306/panel' "$(cat "$ETC/secrets/mysql_password")" > "$ETC/secrets/database_url"
umask 022

# A Docker secret is made once from its file; a file that no longer matches
# it (edited, restored, lost) stops the installer instead of diverging.
docker_secret() { # name
  sum=$(sha256sum "$ETC/secrets/$1" | cut -c1-64)
  if docker secret inspect "rdp_$1" >/dev/null 2>&1; then
    kept=$(docker secret inspect --format '{{index .Spec.Labels "rdockpanel.sha256"}}' "rdp_$1")
    [ -z "$kept" ] || [ "$kept" = "$sum" ] \
      || die "$ETC/secrets/$1 differs from the Docker secret rdp_$1: restore the file (see docs/install.md)"
  else
    docker secret create --label "$SYSTEM" --label "rdockpanel.sha256=$sum" "rdp_$1" "$ETC/secrets/$1" >/dev/null
  fi
}
for s in app_key mysql_root mysql_password database_url; do docker_secret "$s"; done

docker network inspect rdp-internal >/dev/null 2>&1 \
  || docker network create --driver overlay --attachable --label "$SYSTEM" rdp-internal >/dev/null

exists() { docker service inspect "$1" >/dev/null 2>&1; }
constrained() { # service: on this node only (installs of before: on any manager)
  docker service inspect --format '{{join .Spec.TaskTemplate.Placement.Constraints ","}}' "$1" | grep -q "$PLACE" \
    || docker service update --quiet --constraint-rm node.role==manager --constraint-add "$PLACE" "$1" >/dev/null
}

# --- MySQL 8.4 (the panel's database, on rdp-internal only) -------------------
# Alive is enough (a ping answers before credentials; the first start
# initializes the database for a minute or two).
MYSQL_HEALTH='mysqladmin ping -h 127.0.0.1 --silent'
if exists rdp-mysql; then
  constrained rdp-mysql
  docker service update --quiet --image "$MYSQL_IMAGE" \
    --health-cmd "$MYSQL_HEALTH" --health-start-period 120s rdp-mysql >/dev/null
else
  say "Starting MySQL"
  docker volume create --label "$SYSTEM" rdp-mysql-data >/dev/null
  docker service create --quiet --name rdp-mysql \
    --label "$SYSTEM" --container-label "$SYSTEM" \
    --constraint "$PLACE" \
    --network rdp-internal \
    --mount type=volume,source=rdp-mysql-data,target=/var/lib/mysql \
    --secret rdp_mysql_root --secret rdp_mysql_password \
    --env MYSQL_ROOT_PASSWORD_FILE=/run/secrets/rdp_mysql_root \
    --env MYSQL_DATABASE=panel --env MYSQL_USER=panel \
    --env MYSQL_PASSWORD_FILE=/run/secrets/rdp_mysql_password \
    --health-cmd "$MYSQL_HEALTH" --health-interval 5s --health-retries 30 \
    --health-start-period 120s \
    "$MYSQL_IMAGE" \
    --character-set-server=utf8mb4 --collation-server=utf8mb4_0900_bin --default-time-zone=+00:00 \
    >/dev/null
fi

# --- Traefik (ports 80 and 443; Let's Encrypt with --email) -------------------
# The values are checked above: no spaces, so the arguments split as meant.
RESOLVER=""
TRAEFIK_ARGS="--entrypoints.web.address=:80 --entrypoints.websecure.address=:443 \
--providers.file.directory=/etc/traefik/dynamic --providers.file.watch=true --log.level=WARN"
if [ -n "$EMAIL" ]; then
  RESOLVER=letsencrypt
  TRAEFIK_ARGS="$TRAEFIK_ARGS --certificatesresolvers.letsencrypt.acme.email=$EMAIL \
--certificatesresolvers.letsencrypt.acme.storage=/etc/traefik/acme/acme.json \
--certificatesresolvers.letsencrypt.acme.httpchallenge.entrypoint=web"
fi
if exists rdp-traefik; then
  constrained rdp-traefik
  docker service update --quiet --image "$TRAEFIK_IMAGE" --args "$TRAEFIK_ARGS" rdp-traefik >/dev/null
else
  say "Starting Traefik"
  # shellcheck disable=SC2086
  docker service create --quiet --name rdp-traefik \
    --label "$SYSTEM" --container-label "$SYSTEM" \
    --constraint "$PLACE" \
    --network rdp-internal \
    --publish mode=host,published=80,target=80 \
    --publish mode=host,published=443,target=443 \
    --mount type=bind,source="$ETC/traefik/dynamic",target=/etc/traefik/dynamic,readonly \
    --mount type=bind,source="$ETC/traefik/acme",target=/etc/traefik/acme \
    "$TRAEFIK_IMAGE" $TRAEFIK_ARGS >/dev/null
fi

# --- The panel ------------------------------------------------------------------
# With a domain: HTTPS only (no port 3000: passwords never cross plain HTTP).
# Without: http://<ip>:3000, as EasyPanel starts (the installer warns).
if [ -n "$DOMAIN" ]; then
  APP_URL="https://$DOMAIN"
  ORIGINS="[\"https://$DOMAIN\"]"
  # The panel's own route (not a service's: the panel never prunes it).
  cat > "$ETC/traefik/dynamic/rdockpanel.yml" <<YAML
http:
  routers:
    rdockpanel:
      rule: Host(\`$DOMAIN\`)
      entryPoints: [websecure]
      service: rdockpanel
      # Above any service's route (theirs: 10000 + the rule's length at most).
      priority: 1000000
      tls:
        certResolver: $RESOLVER
    rdockpanel-http:
      rule: Host(\`$DOMAIN\`)
      entryPoints: [web]
      service: rdockpanel
      priority: 1000000
      middlewares: [rdockpanel-https]
  middlewares:
    rdockpanel-https:
      redirectScheme: { scheme: https, permanent: true }
  services:
    rdockpanel:
      loadBalancer:
        servers: [{ url: "http://rdockpanel:3000" }]
YAML
else
  APP_URL="http://$ADDRESS:3000"
  ORIGINS="[\"http://$ADDRESS:3000\"]"
  rm -f "$ETC/traefik/dynamic/rdockpanel.yml"
fi

# The environment the installer owns, replaced in one update.
set -- --env-add "APP_URL=$APP_URL" \
  --env-add "RUSTCMS_AUTH__ADMIN_ORIGINS=$ORIGINS" \
  --env-add APP_KEY_FILE=/run/secrets/rdp_app_key \
  --env-add DATABASE_URL_FILE=/run/secrets/rdp_database_url \
  --env-add RUSTCMS_PANEL__TRAEFIK_SERVICE=rdp-traefik \
  --env-add "RUSTCMS_PANEL__CERT_RESOLVER=$RESOLVER" \
  --env-add "RUSTCMS_PANEL__SERVICE_DOMAIN=$APPS_DOMAIN" \
  --env-add "RUSTCMS_PANEL__PUBLIC_IP=$ADDRESS" \
  --env-add HOST_PROC=/host/proc \
  --env-add "RUSTCMS_PANEL__COMPOSE_DIR=$COMPOSE" \
  --env-add RUSTCMS_PANEL__INTERNAL_URL=http://rdockpanel:3000 \
  --env-add RUSTCMS_PANEL__INSTALLED=true

mounted() { docker service inspect --format '{{range .Spec.TaskTemplate.ContainerSpec.Mounts}}{{.Target}} {{end}}' rdockpanel | grep -qw "$1"; }
published() { docker service inspect --format '{{range .Endpoint.Ports}}{{.PublishedPort}} {{end}}' rdockpanel | grep -qw 3000; }
if exists rdockpanel; then
  say "Updating RDockPanel ($IMAGE)"
  constrained rdockpanel
  if [ -n "$DOMAIN" ] && published; then
    set -- "$@" --publish-rm 3000
  elif [ -z "$DOMAIN" ] && ! published; then
    set -- "$@" --publish-add mode=host,published=3000,target=3000
  fi
  mounted "$BACKUPS" || set -- "$@" --mount-add "type=bind,source=$BACKUPS,target=$BACKUPS"
  mounted "$COMPOSE" || set -- "$@" --mount-add "type=bind,source=$COMPOSE,target=$COMPOSE"
  docker service update --quiet --image "$IMAGE" "$@" rdockpanel >/dev/null
else
  say "Starting RDockPanel ($IMAGE)"
  # `service create` takes --env and --publish (not their -add forms).
  env_args=""
  for arg in "$@"; do [ "$arg" = --env-add ] || env_args="$env_args$arg
"; done
  publish=""
  [ -z "$DOMAIN" ] && publish="--publish mode=host,published=3000,target=3000"
  # Each variable as one argument (values have no newlines).
  old_ifs=$IFS
  IFS='
'
  set --
  for value in $env_args; do set -- "$@" --env "$value"; done
  IFS=$old_ifs
  # shellcheck disable=SC2086
  docker service create --quiet --name rdockpanel \
    --label "$SYSTEM" --container-label "$SYSTEM" \
    --constraint "$PLACE" \
    --network rdp-internal $publish \
    --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock \
    --mount type=bind,source="$ETC/traefik/dynamic",target=/etc/rdockpanel/traefik/dynamic \
    --mount type=bind,source=/proc,target=/host/proc,readonly \
    --mount type=bind,source="$BACKUPS",target="$BACKUPS" \
    --mount type=bind,source="$COMPOSE",target="$COMPOSE" \
    --secret rdp_app_key --secret rdp_database_url \
    --health-cmd 'curl -fsS http://127.0.0.1:3000/health' --health-interval 5s --health-retries 60 \
    "$@" "$IMAGE" >/dev/null
fi

say "Waiting for the panel"
panel_container() { docker ps -q --filter "label=com.docker.swarm.service.name=rdockpanel" | head -n1; }
tries=0
until c=$(panel_container) && [ -n "$c" ] \
  && [ "$(docker inspect --format '{{.State.Health.Status}}' "$c" 2>/dev/null)" = healthy ]; do
  tries=$((tries + 1))
  [ "$tries" -ge 150 ] && die "the panel did not start: docker service logs rdockpanel"
  sleep 2
done

# --- The first administrator --------------------------------------------------
if [ -n "$ADMIN_EMAIL" ]; then
  say "Creating the administrator $ADMIN_EMAIL"
  docker exec "$(panel_container)" /app/rdockpanel make:user --email "$ADMIN_EMAIL" --name Admin --super-admin \
    || echo "(an account with that email may exist already)"
fi

say "RDockPanel is ready: $APP_URL/admin"
if [ -z "$DOMAIN" ]; then
  echo "    Plain HTTP: give it a domain (--domain, --email) before using it over the internet."
fi
[ -z "$ADMIN_EMAIL" ] && echo "    Create the administrator: sudo sh install.sh --admin-email you@example.com"
exit 0
