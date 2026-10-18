#!/bin/sh
# Development Swarm for RDockPanel (idempotent: run it again any time).
#
# - turns this Docker engine into a single-node Swarm (leave it with
#   `docker swarm leave --force`; existing containers keep running);
# - runs Traefik v3 as the service `rdp-traefik` on ports 8080 (HTTP) and
#   8443 (HTTPS, Traefik's default certificate), reading routes from
#   ./storage/traefik/dynamic (the panel writes them there in development).
#
# Then http://<name>.localhost:8080 reaches a deployed service with that domain.
set -eu

cd "$(dirname "$0")/.."
DYNAMIC="$(pwd)/storage/traefik/dynamic"
TRAEFIK_IMAGE="traefik:v3.5"
SYSTEM_LABEL="rdockpanel.system=true"

state="$(docker info --format '{{.Swarm.LocalNodeState}}')"
if [ "$state" != "active" ]; then
  echo "Initializing a single-node Swarm…"
  docker swarm init --advertise-addr 127.0.0.1 >/dev/null
fi

mkdir -p "$DYNAMIC"

if docker service inspect rdp-traefik >/dev/null 2>&1; then
  echo "rdp-traefik is already running."
else
  echo "Starting rdp-traefik (ports 8080, 8443)…"
  docker service create \
    --name rdp-traefik \
    --label "$SYSTEM_LABEL" \
    --container-label "$SYSTEM_LABEL" \
    --constraint node.role==manager \
    --publish mode=host,published=8080,target=80 \
    --publish mode=host,published=8443,target=443 \
    --mount "type=bind,source=$DYNAMIC,target=/etc/traefik/dynamic,readonly" \
    "$TRAEFIK_IMAGE" \
    --entrypoints.web.address=:80 \
    --entrypoints.websecure.address=:443 \
    --providers.file.directory=/etc/traefik/dynamic \
    --providers.file.watch=true \
    --log.level=INFO >/dev/null
fi

echo "Swarm: $(docker info --format '{{.Swarm.LocalNodeState}}'), routes: $DYNAMIC"
