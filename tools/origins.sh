#!/usr/bin/env bash
# The far-origin check (issue #93; docs/PROCESS.md): the city's south view and the ballad's fixed
# step captured with the scene moved 10^4 to 10^7 m from the world's origin (`--origin`), each
# compared with the same view at the origin in pixels and in LDR-FLIP (issue #75), with the
# difference and the error map written beside it. The scene, its camera and everything anchored
# to them move together, so a renderer without a precision limit would draw the same image at
# every offset; what differs is the f32 error of world-space positions, which D-004's amendment
# (integer cells) proposes to remove. Differences are expected here: the exit code is 0 unless a
# capture or a comparison failed to run.
#
#   tools/origins.sh OUT [BIN] [ORIGINS]
#
# OUT: the directory to write (created). BIN: the directory holding the demo binaries (default
# target/release). ORIGINS: the offsets in metres, quoted (default "10000 100000 1000000
# 10000000"). The demos open on the secondary monitor and never take focus (FORGE_MONITOR); each
# prints the f32 spacing of a position at its scene's far edge.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:?usage: tools/origins.sh OUT [BIN] [ORIGINS]}
bin=${2:-$root/target/release}
origins=${3:-"10000 100000 1000000 10000000"}
mkdir -p "$out"
exe=""
[ -f "$bin/asteroids.exe" ] && exe=.exe
asteroids=$bin/asteroids$exe
city=$bin/city-blocks$exe
imgdiff=$root/target/release/imgdiff$exe
for tool in "$asteroids" "$city" "$imgdiff"; do
  [ -f "$tool" ] || { echo "missing $tool: build with cargo build --release" >&2; exit 1; }
done
cd "$root"

status=0
# Runs a demo, printing its errors and panics and the line on the scene's offset (colour codes
# stripped).
run() {
  "$@" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E "ERROR|Error|panicked|world's origin" || true
}
# capture NAME FRAME DEMO ARGS...: the frame FRAME of DEMO to OUT/NAME.png.
capture() {
  local name=$1 frame=$2 demo=$3
  shift 3
  run "$demo" --frames $((frame + 1)) --capture "$out/$name.png" --capture-frame "$frame" "$@"
  [ -f "$out/$name.png" ] || { echo "$name: no capture" >&2; status=1; }
}
# pair BASE NEW LABEL: the pixels that differ, the largest channel error and, for a difference,
# LDR-FLIP's mean and largest value; the difference and the error map beside NEW.
pair() {
  [ -f "$1" ] && [ -f "$2" ] || return 0
  local stem=${2%.png} out line flip count max mean peak
  out=$("$imgdiff" "$1" "$2" --out "$stem-diff.png" --flip-map "$stem-flip.png" 2>&1)
  line=$(grep "pixels differ" <<< "$out" | tail -n 1)
  flip=$(grep "LDR-FLIP" <<< "$out" | tail -n 1)
  count=$(sed -n 's/.*: \([0-9]*\) \/ [0-9]* pixels differ.*/\1/p' <<< "$line")
  max=$(sed -n 's/.*max channel error \([0-9]*\).*/\1/p' <<< "$line")
  mean=$(sed -n 's/.*: mean \([0-9.]*\),.*/\1/p' <<< "$flip")
  peak=$(sed -n 's/.* max \([0-9.]*\) at .*/\1/p' <<< "$flip")
  if [ -z "$count" ]; then
    echo "$3: imgdiff failed"
    status=1
  elif [ "$count" != 0 ]; then
    echo "$3: $count px differ (max $max), FLIP mean $mean, max $peak"
  else
    echo "$3: 0 px"
  fi
}

# The same views as the batch's city60 and ast240 (tools/captures.sh), every page resident, no
# TAA on the ballad: the raw frame shows where the geometry moved.
for origin in 0 $origins; do
  capture "city-o$origin" 60 "$city" --stream-pool 0 --origin "$origin"
  capture "ast-o$origin" 240 "$asteroids" --fixed-step --no-taa --origin "$origin"
done
echo "== against the same view at the origin"
for origin in $origins; do
  pair "$out/city-o0.png" "$out/city-o$origin.png" "city at $origin m"
  pair "$out/ast-o0.png" "$out/ast-o$origin.png" "ballad at $origin m"
done
exit $status
