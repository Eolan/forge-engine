#!/usr/bin/env bash
# The hazard check (the audit's §1.2, 2026-10-25): finds which synchronisation a race depends
# on by running one scripted view many times under each of the render graph's debugging
# variables and comparing the per-frame image hashes (`FORGE_HASH_IMAGES=1`, #161) across the
# runs. A pass that comes out one of two ways from the same inputs (#207, #349) gives two
# distinct hash lines over the runs; the variable under which every run agrees names the
# class of the hazard: `FORGE_GRAPH_NO_ALIAS` an aliased transient read before its occupant
# was done, `FORGE_GRAPH_ZERO` a read of texels nothing wrote this frame, `FORGE_GRAPH_NO_ASYNC`
# or `FORGE_ASYNC=0` a wait between queues, `FORGE_FULL_BARRIERS_BEFORE` a pass's own barrier.
#
#   tools/hazard-check.sh [-n RUNS] [-f FRAME] [--dry-run] -- DEMO ARGS...
#
#   tools/hazard-check.sh -n 8 -f 120 -- target/release/physics-lab.exe --lab materials \
#       --fixed-step --frames 121
#
# RUNS: runs per variable (6 by default). FRAME: the frame whose hashes are compared (the last
# logged one by default). The demo must be a gallery (city-blocks, physics-lab, city-plan, the
# island), which log their hashes, at a fixed step with a frame limit. FORGE_HAZARD_WORDS gives
# the words for FORGE_FULL_BARRIERS_BEFORE ("mirror" by default: #349's pass). The runs' logs
# stay in captures/hazard/<time>/ with the table, hazard.txt. Under the GPU lock.
# The owner's settings file (forge.cfg, D-069) never changes a scripted run.
export FORGE_CFG=none
export FORGE_HASH_IMAGES=1
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
runs=6
frame=""
dry=0
while [ $# -gt 0 ]; do
  case $1 in
    -n) runs=${2:?-n RUNS}; shift 2 ;;
    -f) frame=${2:?-f FRAME}; shift 2 ;;
    --dry-run) dry=1; shift ;;
    --) shift; break ;;
    *) echo "usage: tools/hazard-check.sh [-n RUNS] [-f FRAME] [--dry-run] -- DEMO ARGS..." >&2; exit 2 ;;
  esac
done
[ $# -gt 0 ] || { echo "usage: tools/hazard-check.sh [-n RUNS] [-f FRAME] [--dry-run] -- DEMO ARGS..." >&2; exit 2; }
words=${FORGE_HAZARD_WORDS:-mirror}
# Each variable set, as "NAME=VALUE" (the baseline sets none).
configs=(
  "baseline"
  "FORGE_GRAPH_NO_ALIAS=1"
  "FORGE_GRAPH_ZERO=1"
  "FORGE_GRAPH_NO_ASYNC=1"
  "FORGE_ASYNC=0"
  "FORGE_SERIAL_BARRIERS=0"
  "FORGE_FULL_BARRIERS_BEFORE=$words"
)
if [ "$dry" = 1 ]; then
  echo "would run '$*' $runs times under each of:"
  printf '  %s\n' "${configs[@]}"
  echo "and compare the image hashes of frame ${frame:-(the last logged)}"
  exit 0
fi

if [ -n "${FORGE_GPU_LOCK:-}" ]; then
  lock=$FORGE_GPU_LOCK
elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-gpu.lock
else
  lock=${TMPDIR:-/tmp}/forge-gpu.lock
fi
stamp=$(date +%Y%m%d-%H%M%S)
out=$root/captures/hazard/$stamp
mkdir -p "$out"
table=$out/hazard.txt
echo "== waiting for the GPU lock ($lock)"
until mkdir "$lock" 2> /dev/null; do sleep 5; done
echo "forge hazard-check.sh" > "$lock/owner"
trap 'rm -f "$lock/owner"; rmdir "$lock" 2> /dev/null' EXIT

# hashes_of LOG: the hash line of the frame compared, as "hashes=[...]", or "none".
hashes_of() {
  local log=$1 line
  if [ -n "$frame" ]; then
    line=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -E "image hashes frame=$frame " | tail -1)
  else
    line=$(sed 's/\x1b\[[0-9;]*m//g' "$log" | grep -E "image hashes frame=" | tail -1)
  fi
  [ -n "$line" ] || { echo none; return; }
  echo "$line" | grep -oE 'hashes=.*$'
}

{
  echo "hazard check $stamp: '$*', $runs runs each, frame ${frame:-(the last logged)}"
  printf '%-36s %8s  %s\n' variable distinct "runs apart from the first (none: every run agrees)"
} > "$table"
suspect=0
for config in "${configs[@]}"; do
  declare -A seen=()
  apart=""
  first=""
  for ((k = 1; k <= runs; k++)); do
    log=$out/$config-$k.log
    if [ "$config" = baseline ]; then
      FORGE_MONITOR=${FORGE_MONITOR:-secondary} timeout 600 "$@" > "$log" 2>&1
    else
      FORGE_MONITOR=${FORGE_MONITOR:-secondary} env "$config" timeout 600 "$@" > "$log" 2>&1
    fi
    h=$(hashes_of "$log")
    seen["$h"]=1
    [ -n "$first" ] || first=$h
    [ "$h" = "$first" ] || apart="$apart $k"
    echo "$config run $k: $(echo "$h" | cut -c1-72)"
  done
  distinct=${#seen[@]}
  [ "$config" = baseline ] && [ "$distinct" -gt 1 ] && suspect=1
  printf '%-36s %8s  %s\n' "$config" "$distinct" "${apart:-none}" >> "$table"
  unset seen
done
rm -f "$lock/owner"; rmdir "$lock" 2> /dev/null; trap - EXIT
cat "$table"
if [ "$suspect" = 1 ]; then
  echo "== the baseline parts: the variable with 1 distinct line names the hazard's class"
else
  echo "== every baseline run agrees: no race seen in $runs runs (try more, or another view)"
fi
