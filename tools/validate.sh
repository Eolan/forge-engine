#!/usr/bin/env bash
# Runs every demo on both paths under the Vulkan validation layer, synchronization validation
# included (issue #74; docs/PROCESS.md), and prints each run's messages, counted. A clean run
# prints its header line only. The GOG overlay layer's naming warnings are noise and dropped.
#
#   tools/validate.sh [BIN]
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-$root/target/release}
exe=""
[ -f "$bin/asteroids.exe" ] && exe=.exe
cd "$root"

validate() {
  echo "== $*"
  FORGE_SYNC_VALIDATION=1 "$@" --validate 2>&1 | sed 's/\x1b\[[0-9;]*m//g' |
    grep -E "VUID|SYNC-|Validation (Error|Warning)|ERROR|panicked|mip check" | grep -v "GOG" |
    cut -c1-300 | sort | uniq -c | head -n 8
}

for path in "" --force-fallback; do
  validate "$bin/asteroids$exe" --frames 90 $path
  validate "$bin/meshlets$exe" --mip-check --frames 60 $path
  validate "$bin/city-blocks$exe" --frames 90 $path
  validate "$bin/city-blocks$exe" --stream-pool 0 --frames 60 $path
  validate "$bin/city-blocks$exe" --gallery --frames 60 $path
done
