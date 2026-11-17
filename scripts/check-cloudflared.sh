#!/bin/sh
# Compares the cloudflared the panel pins (src/services/tunnel_service.rs) with
# Cloudflare's newest release on Docker Hub. Run before every release: exit 1
# when a newer one exists (bump IMAGE, then read its release notes).
set -eu
cd "$(dirname "$0")/.."
pinned=$(sed -n 's|^pub const IMAGE: &str = "cloudflare/cloudflared:\([^"]*\)";|\1|p' src/services/tunnel_service.rs)
[ -n "$pinned" ] || { echo "check-cloudflared: IMAGE not found" >&2; exit 2; }
latest=$(curl -fsSL "https://hub.docker.com/v2/repositories/cloudflare/cloudflared/tags?page_size=100&ordering=last_updated" \
  | grep -o '"name": *"[0-9]\{4\}\.[0-9]\{1,2\}\.[0-9]\{1,3\}"' | sed 's/.*"\([^"]*\)"$/\1/' \
  | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)
[ -n "$latest" ] || { echo "check-cloudflared: Docker Hub gave no release tag" >&2; exit 2; }
if [ "$pinned" = "$latest" ]; then
  echo "cloudflared $pinned: the newest"
else
  echo "cloudflared $pinned pinned; $latest is out: bump IMAGE in src/services/tunnel_service.rs"
  echo "  notes: https://github.com/cloudflare/cloudflared/releases/tag/$latest"
  exit 1
fi
