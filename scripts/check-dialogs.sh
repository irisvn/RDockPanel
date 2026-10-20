#!/bin/sh
# The browser's own dialogs (confirm, alert, prompt) are being replaced by the
# panel's (docs/ui-guidelines.md §6): their count in the panel's sources may
# only go down. Lower MAX here as pages move; it ends at 0.
set -eu
MAX=7
cd "$(dirname "$0")/../resources/admin"
# The panel's own sources: React pages and the plain scripts, not the built
# bundle (js/rdockpanel/dashboard.js) nor vendored libraries.
# Each call counts, two on one line too.
search() {
  grep -rE "$@" 'window\.(confirm|alert|prompt)\(' src js/rdockpanel \
    --include='*.ts' --include='*.tsx' --include='*.js' \
    --exclude-dir=vendor --exclude=dashboard.js || true
}
found=$(search -n)
count=$(search -o | grep -c . || true)
if [ "$count" -gt "$MAX" ]; then
  printf '%s\n' "$found"
  echo "browser dialogs: $count, at most $MAX allowed (use ConfirmDialog)" >&2
  exit 1
fi
echo "browser dialogs: $count (at most $MAX)"
