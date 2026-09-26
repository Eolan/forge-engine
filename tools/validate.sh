#!/usr/bin/env bash
# Runs every demo on both paths under the Vulkan validation layer, synchronization validation
# included (issue #74; docs/PROCESS.md), and prints each run's messages, counted. A clean run
# prints its header line, its duration and its log's length only. The GOG overlay layer's
# naming warnings are noise and dropped. The runs are short on purpose (60–90 frames each, a
# minute or two in all): the layer's cost is in the checks, not the frames.
#
#   tools/validate.sh [BIN] [OUT]
#
# BIN: the demo binaries (default target/release). With FORGE_KEEP_LOGS=1 each run's full log
# is kept in OUT/logs/ (default captures/validate) and what was printed in OUT/summary.txt,
# for a cloud session to read (tools/report.sh gathers them).
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-$root/target/release}
out=${2:-$root/captures/validate}
keep=${FORGE_KEEP_LOGS:-0}
summary=$out/summary.txt
exe=""
[ -f "$bin/asteroids.exe" ] && exe=.exe
cd "$root"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
header="tools/validate.sh $bin, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD 2>/dev/null)"
echo "$header"
if [ "$keep" != 0 ]; then
  mkdir -p "$out/logs"
  echo "$header" > "$summary"
fi
# say LINE...: prints, and keeps in the summary with FORGE_KEEP_LOGS=1.
say() {
  echo "$@"
  [ "$keep" != 0 ] && echo "$@" >> "$summary"
  return 0
}

# validate NAME CMD...: the run under the layer, its messages counted.
validate() {
  local name=$1
  shift
  local start=$SECONDS
  say "== $*"
  FORGE_SYNC_VALIDATION=1 "$@" --validate 2>&1 | sed 's/\x1b\[[0-9;]*m//g' > "$log"
  while IFS= read -r line; do say "$line"; done < <(
    grep -E "VUID|SYNC-|Validation (Error|Warning)|ERROR|panicked|mip check|tone check" "$log" |
      grep -v "GOG" | cut -c1-300 | sort | uniq -c | head -n 8)
  say "   $((SECONDS - start)) s, $(wc -l < "$log") log lines"
  [ -s "$log" ] || say "   the run printed nothing: did the demo start?"
  if [ "$keep" != 0 ]; then
    cp "$log" "$out/logs/$name.log"
    grep -E "forge_app: gpu:|selected GPU" "$log" | cut -c1-200 >> "$summary"
  fi
}

for path in "" --force-fallback; do
  tag=${path:+-fb}
  validate "ballad$tag" "$bin/asteroids$exe" --frames 90 $path
  validate "meshlets$tag" "$bin/meshlets$exe" --mip-check --tone-check --frames 60 $path
  validate "city$tag" "$bin/city-blocks$exe" --frames 90 $path
  validate "city-resident$tag" "$bin/city-blocks$exe" --stream-pool 0 --frames 60 $path
  validate "gallery$tag" "$bin/city-blocks$exe" --gallery --frames 60 $path
  validate "island$tag" "$bin/city-blocks$exe" --island 7 --stream-pool 0 --frames 60 $path
done
[ "$keep" != 0 ] && say "logs in $out/logs, summary in $summary"
exit 0
