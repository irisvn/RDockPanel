#!/bin/sh
# The panel's container: migrations (waiting for MySQL while it starts),
# permissions, then the command (`serve` by default).
set -eu
cd /app
if [ "${1:-serve}" = "serve" ]; then
  tries=0
  until /app/rdockpanel migrate; do
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ]; then
      echo "rdockpanel: the database is not reachable" >&2
      exit 1
    fi
    sleep 2
  done
  /app/rdockpanel permission:sync
fi
exec /app/rdockpanel "${@:-serve}"
