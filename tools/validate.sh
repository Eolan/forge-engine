#!/usr/bin/env bash
# Runs every demo on both paths under the Vulkan validation layer, synchronization validation
# included (issue #74; docs/PROCESS.md), and prints each run's messages, counted. A clean run
# prints its header line, its duration and its log's length only. The GOG overlay layer's
# naming warnings are noise and dropped.
#
#   tools/validate.sh [BIN] [OUT]
#
# BIN: the demo binaries (default target/release). OUT (default captures/validate): each run's
# full log in OUT/logs/ and OUT/summary.txt with what was printed, for a cloud session to read
# (tools/report.sh gathers them). The runs are short on purpose (60–90 frames each, a minute or
# two in all): the layer's cost is in the checks, not the frames.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-$root/target/release}
out=${2:-$root/captures/validate}
mkdir -p "$out/logs"
summary=$out/summary.txt
exe=""
[ -f "$bin/asteroids.exe" ] && exe=.exe
cd "$root"
echo "tools/validate.sh $bin, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD 2>/dev/null)" | tee "$summary"

# validate NAME CMD...: the run under the layer, its log to OUT/logs/NAME.log, its messages
# counted (the same lines printed and kept in the summary).
validate() {
  local name=$1
  shift
  local log=$out/logs/$name.log start=$SECONDS
  echo "== $*" | tee -a "$summary"
  FORGE_SYNC_VALIDATION=1 "$@" --validate 2>&1 | sed 's/\x1b\[[0-9;]*m//g' > "$log"
  grep -E "VUID|SYNC-|Validation (Error|Warning)|ERROR|panicked|mip check|tone check" "$log" | grep -v "GOG" |
    cut -c1-300 | sort | uniq -c | head -n 8 | tee -a "$summary"
  echo "   $((SECONDS - start)) s, $(wc -l < "$log") log lines ($log)" | tee -a "$summary"
  grep -E "forge_app: gpu:|selected GPU" "$log" | cut -c1-200 >> "$summary"
  [ -s "$log" ] || echo "   the run printed nothing: did the demo start?" | tee -a "$summary"
}

for path in "" --force-fallback; do
  tag=${path:+-fb}
  validate "ballad$tag" "$bin/asteroids$exe" --frames 90 $path
  validate "meshlets$tag" "$bin/meshlets$exe" --mip-check --tone-check --frames 60 $path
  validate "city$tag" "$bin/city-blocks$exe" --frames 90 $path
  validate "city-resident$tag" "$bin/city-blocks$exe" --stream-pool 0 --frames 60 $path
  validate "gallery$tag" "$bin/city-blocks$exe" --gallery --frames 60 $path
done
echo "logs in $out/logs, summary in $summary" | tee -a "$summary"
