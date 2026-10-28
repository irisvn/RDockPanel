#!/bin/sh
# The panel's container: migrations (waiting for MySQL while it starts;
# SQLite is a file, there at once),
# permissions, then the command (`serve` by default). `--allow-newer`: a
# database a newer version migrated is used as it is (Settings → Version's
# Update rolled back to this one); the panel's migrations only add.
set -eu
cd /app
if [ "${1:-serve}" = "serve" ]; then
  # While MySQL starts (after a reboot it may come up after the panel):
  # one line per try, its error only when giving up.
  tries=0
  until out=$(/app/rdockpanel migrate --allow-newer 2>&1); do
    tries=$((tries + 1))
    if [ "$tries" -ge 60 ]; then
      printf '%s\n' "$out" >&2
      echo "rdockpanel: the database is not reachable" >&2
      exit 1
    fi
    echo "rdockpanel: waiting for the database to start ($tries)" >&2
    sleep 2
  done
  printf '%s\n' "$out"
  /app/rdockpanel permission:sync
fi
exec /app/rdockpanel "${@:-serve}"
