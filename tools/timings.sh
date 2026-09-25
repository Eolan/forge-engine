#!/usr/bin/env bash
# GPU frame times of the usual views, a baseline build against the current one (issue #74;
# docs/PROCESS.md). Each view runs three times per build, alternating, so a warm or throttled
# GPU weighs on both alike; the demos print their average GPU time and zones when they exit.
#
#   tools/timings.sh BASE_BIN [NEW_BIN] [ZONES]
#
# BASE_BIN, NEW_BIN: directories holding the demo binaries (NEW_BIN defaults to
# target/release). ZONES: an extended regex; the matching GPU zones of each build's last run
# are printed under its times (for example 'cull' or 'gi/'). About ten minutes. With
# FORGE_KEEP_LOGS=1 every run's log is kept under TIMINGS_OUT/logs/ (default captures/timings)
# and the printed lines in TIMINGS_OUT/summary.txt, for a cloud session to read
# (tools/report.sh gathers them).
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
base=${1:?usage: tools/timings.sh BASE_BIN [NEW_BIN] [ZONES]}
new=${2:-$root/target/release}
zones=${3:-}
keep=${FORGE_KEEP_LOGS:-0}
out=${TIMINGS_OUT:-$root/captures/timings}
summary=$out/summary.txt
exe=""
[ -f "$base/asteroids.exe" ] && exe=.exe
cd "$root"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
header="tools/timings.sh base $base, new $new, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD 2>/dev/null)"
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

# The exit line reads "forge_app: gpu: X ms ... zone Y, ..."; strip colours first.
gpu() { sed 's/\x1b\[[0-9;]*m//g' "$log" | grep "forge_app: gpu:" | sed 's/.*gpu: \([0-9.]*\) ms.*/\1/'; }
zone_list() {
  [ -n "$zones" ] || return 0
  sed 's/\x1b\[[0-9;]*m//g' "$log" | grep "forge_app: gpu:" | sed 's/.* frames: //' | tr ',' '\n' |
    grep -E "$zones" |
    sed 's/^ *//' | tr '\n' ';'
}
# keep_log NAME: the run's log under TIMINGS_OUT/logs with FORGE_KEEP_LOGS=1.
keep_log() {
  [ "$keep" != 0 ] && sed 's/\x1b\[[0-9;]*m//g' "$log" > "$out/logs/$1.log"
  return 0
}
view() {
  local name=$1 demo=$2
  shift 2
  local slug=${name// /-} old="" now="" old_zones="" new_zones=""
  for i in 1 2 3; do
    "$base/$demo$exe" "$@" > "$log" 2>&1
    keep_log "$slug-base-$i"
    old="$old $(gpu)"
    old_zones=$(zone_list)
    "$new/$demo$exe" "$@" > "$log" 2>&1
    keep_log "$slug-new-$i"
    now="$now $(gpu)"
    new_zones=$(zone_list)
  done
  say "$name | base:$old | new:$now"
  [ -n "$zones" ] && say "   base: $old_zones" && say "   new:  $new_zones"
  return 0
}

view "city" city-blocks --frames 3000
view "city orbit" city-blocks --orbit --frames 3000
view "city fly" city-blocks --fly --frames 6000
view "city resident" city-blocks --stream-pool 0 --frames 3000
view "meshlets" meshlets --frames 3000
view "meshlets orbit" meshlets --orbit --frames 3000
view "meshlets side 700" meshlets --side 700 --frames 600
view "ballad 900p" asteroids --fixed-step --frames 3000
view "ballad 1440p" asteroids --fixed-step --frames 3000 --width 2560 --height 1440
[ "$keep" != 0 ] && say "logs in $out/logs, summary in $summary"
exit 0
