#!/bin/sh
# RDockPanel installer, for a fresh Linux server (Ubuntu or Debian):
#
#   curl -fsSL https://raw.githubusercontent.com/irisvn/rdockpanel/main/install/install.sh | sudo sh
#   curl -fsSL …/install.sh | sudo sh -s -- --domain panel.example.com --email you@example.com \
#       --apps-domain apps.example.com --admin-email you@example.com
#
# Options:
#   --domain HOST        the panel at https://HOST (Let's Encrypt, with --email)
#   --email ADDRESS      Let's Encrypt account: HTTPS certificates for the panel and the apps
#   --apps-domain HOST   generated app domains <project>-<service>.HOST (a wildcard DNS record)
#   --admin-email EMAIL  creates the first administrator (prints its password)
#   --image IMAGE        the panel's image (default ghcr.io/irisvn/rdockpanel:latest)
#
# Run it again any time (to upgrade, or to change the options): what exists
# is kept, its secrets first (APP_KEY, the database's passwords).
set -eu

IMAGE="ghcr.io/irisvn/rdockpanel:latest"
DOMAIN=""
EMAIL=""
APPS_DOMAIN=""
ADMIN_EMAIL=""
while [ $# -gt 0 ]; do
  case "$1" in
    --domain) DOMAIN="$2"; shift 2 ;;
    --email) EMAIL="$2"; shift 2 ;;
    --apps-domain) APPS_DOMAIN="$2"; shift 2 ;;
    --admin-email) ADMIN_EMAIL="$2"; shift 2 ;;
    --image) IMAGE="$2"; shift 2 ;;
    -h|--help) sed -n '2,16p' "$0" 2>/dev/null || true; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf 'rdockpanel: %s\n' "$*" >&2; exit 1; }

[ "$(id -u)" = 0 ] || die "run it as root (sudo)"
[ "$(uname -s)" = Linux ] || die "RDockPanel installs on Linux"

ETC=/etc/rdockpanel
SYSTEM=rdockpanel.system=true

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

# --- Directories and secrets (made once, kept) -------------------------------
mkdir -p "$ETC/secrets" "$ETC/traefik/dynamic" "$ETC/traefik/acme"
chmod 0700 "$ETC/secrets" "$ETC/traefik/acme"
touch "$ETC/traefik/acme/acme.json"
chmod 0600 "$ETC/traefik/acme/acme.json"

secret_file() { # name generator
  if [ ! -s "$ETC/secrets/$1" ]; then
    umask 077
    eval "$2" > "$ETC/secrets/$1"
  fi
}
random() { head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n' | cut -c1-32; }
secret_file app_key 'printf "base64:%s" "$(head -c 32 /dev/urandom | base64 | tr -d "\n")"'
secret_file mysql_root random
secret_file mysql_password random
printf 'mysql://panel:%s@rdp-mysql:3306/panel' "$(cat "$ETC/secrets/mysql_password")" > "$ETC/secrets/database_url"
chmod 0600 "$ETC/secrets/database_url"

docker_secret() { # name
  docker secret inspect "rdp_$1" >/dev/null 2>&1 \
    || docker secret create --label "$SYSTEM" "rdp_$1" "$ETC/secrets/$1" >/dev/null
}
for s in app_key mysql_root mysql_password database_url; do docker_secret "$s"; done

docker network inspect rdp-internal >/dev/null 2>&1 \
  || docker network create --driver overlay --attachable --label "$SYSTEM" rdp-internal >/dev/null

exists() { docker service inspect "$1" >/dev/null 2>&1; }

# --- MySQL 8.4 (the panel's database, on rdp-internal only) -------------------
# Alive is enough (a ping answers before credentials; the first start
# initializes the database for a minute or two).
MYSQL_HEALTH='mysqladmin ping -h 127.0.0.1 --silent'
if exists rdp-mysql; then
  docker service update --quiet --health-cmd "$MYSQL_HEALTH" --health-start-period 120s rdp-mysql >/dev/null
fi
if ! exists rdp-mysql; then
  say "Starting MySQL"
  docker volume create --label "$SYSTEM" rdp-mysql-data >/dev/null
  docker service create --quiet --name rdp-mysql \
    --label "$SYSTEM" --container-label "$SYSTEM" \
    --constraint node.role==manager \
    --network rdp-internal \
    --mount type=volume,source=rdp-mysql-data,target=/var/lib/mysql \
    --secret rdp_mysql_root --secret rdp_mysql_password \
    --env MYSQL_ROOT_PASSWORD_FILE=/run/secrets/rdp_mysql_root \
    --env MYSQL_DATABASE=panel --env MYSQL_USER=panel \
    --env MYSQL_PASSWORD_FILE=/run/secrets/rdp_mysql_password \
    --health-cmd "$MYSQL_HEALTH" --health-interval 5s --health-retries 30 \
    --health-start-period 120s \
    mysql:8.4 \
    --character-set-server=utf8mb4 --collation-server=utf8mb4_0900_bin --default-time-zone=+00:00 \
    >/dev/null
fi

# --- Traefik (ports 80 and 443; Let's Encrypt with --email) -------------------
RESOLVER=""
TRAEFIK_ARGS="--entrypoints.web.address=:80 --entrypoints.websecure.address=:443 \
--providers.file.directory=/etc/traefik/dynamic --providers.file.watch=true --log.level=WARN"
if [ -n "$EMAIL" ]; then
  RESOLVER=letsencrypt
  TRAEFIK_ARGS="$TRAEFIK_ARGS --certificatesresolvers.letsencrypt.acme.email=$EMAIL \
--certificatesresolvers.letsencrypt.acme.storage=/etc/traefik/acme/acme.json \
--certificatesresolvers.letsencrypt.acme.httpchallenge.entrypoint=web"
fi
if ! exists rdp-traefik; then
  say "Starting Traefik"
  # shellcheck disable=SC2086
  docker service create --quiet --name rdp-traefik \
    --label "$SYSTEM" --container-label "$SYSTEM" \
    --constraint node.role==manager \
    --network rdp-internal \
    --publish mode=host,published=80,target=80 \
    --publish mode=host,published=443,target=443 \
    --mount type=bind,source="$ETC/traefik/dynamic",target=/etc/traefik/dynamic,readonly \
    --mount type=bind,source="$ETC/traefik/acme",target=/etc/traefik/acme \
    traefik:v3.5 $TRAEFIK_ARGS >/dev/null
else
  # shellcheck disable=SC2086
  docker service update --quiet --args "$TRAEFIK_ARGS" rdp-traefik >/dev/null
fi

# --- The panel ------------------------------------------------------------------
if [ -n "$DOMAIN" ]; then
  APP_URL="https://$DOMAIN"
  ORIGINS="[\"https://$DOMAIN\",\"http://$ADDRESS:3000\"]"
  TLS=""
  [ -n "$RESOLVER" ] && TLS="
      tls:
        certResolver: $RESOLVER"
  # The panel's own route (not a service's: the panel never prunes it).
  cat > "$ETC/traefik/dynamic/rdockpanel.yml" <<YAML
http:
  routers:
    rdockpanel:
      rule: Host(\`$DOMAIN\`)
      entryPoints: [websecure]
      service: rdockpanel$TLS
    rdockpanel-http:
      rule: Host(\`$DOMAIN\`)
      entryPoints: [web]
      service: rdockpanel
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

PANEL_ENV="--env APP_URL=$APP_URL \
--env RUSTCMS_AUTH__ADMIN_ORIGINS=$ORIGINS \
--env APP_KEY_FILE=/run/secrets/rdp_app_key \
--env DATABASE_URL_FILE=/run/secrets/rdp_database_url \
--env RUSTCMS_PANEL__TRAEFIK_SERVICE=rdp-traefik \
--env RUSTCMS_PANEL__CERT_RESOLVER=$RESOLVER \
--env RUSTCMS_PANEL__SERVICE_DOMAIN=$APPS_DOMAIN \
--env RUSTCMS_PANEL__PUBLIC_IP=$ADDRESS \
--env HOST_PROC=/host/proc"

if ! exists rdockpanel; then
  say "Starting RDockPanel ($IMAGE)"
  # shellcheck disable=SC2086
  docker service create --quiet --name rdockpanel \
    --label "$SYSTEM" --container-label "$SYSTEM" \
    --constraint node.role==manager \
    --network rdp-internal \
    --publish mode=host,published=3000,target=3000 \
    --mount type=bind,source=/var/run/docker.sock,target=/var/run/docker.sock \
    --mount type=bind,source="$ETC/traefik/dynamic",target=/etc/rdockpanel/traefik/dynamic \
    --mount type=bind,source=/proc,target=/host/proc,readonly \
    --secret rdp_app_key --secret rdp_database_url \
    --health-cmd 'curl -fsS http://127.0.0.1:3000/health' --health-interval 5s --health-retries 60 \
    $PANEL_ENV "$IMAGE" >/dev/null
else
  say "Updating RDockPanel ($IMAGE)"
  # Replaces the environment the installer owns; the rest is kept.
  for name in APP_URL RUSTCMS_AUTH__ADMIN_ORIGINS RUSTCMS_PANEL__CERT_RESOLVER RUSTCMS_PANEL__SERVICE_DOMAIN RUSTCMS_PANEL__PUBLIC_IP; do
    docker service update --quiet --env-rm "$name" rdockpanel >/dev/null
  done
  # shellcheck disable=SC2086
  docker service update --quiet --image "$IMAGE" $(echo "$PANEL_ENV" | sed 's/--env /--env-add /g') rdockpanel >/dev/null
fi

say "Waiting for the panel"
tries=0
until curl -fsS "http://127.0.0.1:3000/health" >/dev/null 2>&1; do
  tries=$((tries + 1))
  [ "$tries" -ge 120 ] && die "the panel did not start: docker service logs rdockpanel"
  sleep 2
done

# --- The first administrator --------------------------------------------------
if [ -n "$ADMIN_EMAIL" ]; then
  container=$(docker ps -q --filter "label=com.docker.swarm.service.name=rdockpanel" | head -n1)
  say "Creating the administrator $ADMIN_EMAIL"
  docker exec "$container" /app/rdockpanel make:user --email "$ADMIN_EMAIL" --name Admin --super-admin \
    || echo "(an account with that email may exist already)"
fi

say "RDockPanel is ready: $APP_URL/admin"
[ -n "$DOMAIN" ] && echo "    (also at http://$ADDRESS:3000/admin until $DOMAIN points here)"
[ -z "$ADMIN_EMAIL" ] && echo "    Create the administrator: sudo sh install.sh --admin-email you@example.com"
exit 0
