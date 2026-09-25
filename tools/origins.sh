#!/usr/bin/env bash
# The far-origin check (issue #93; docs/PROCESS.md): the city's south view and the ballad's fixed
# step captured with the scene moved 10^4 to 10^7 m from the world's origin (`--origin`), each
# compared with the same view at the origin in pixels and in LDR-FLIP (issue #75), with the
# difference and the error map written beside it. The scene, its camera and everything anchored
# to them move together, so a renderer without a precision limit would draw the same image at
# every offset; what differs is the f32 error of world-space positions, which D-004's amendment
# (integer cells) proposes to remove. Differences are expected until then: the exit code is 0
# unless a capture or a comparison failed to run.
#
#   tools/origins.sh OUT [BIN] [ORIGINS]
#
# OUT: the directory to write (created): the captures, each run's full log in OUT/logs/ and
# OUT/summary.txt with the comparison lines (tools/report.sh gathers them to commit). BIN: the
# directory holding the demo binaries (default target/release). ORIGINS: the offsets in metres,
# quoted (default "10000 100000 1000000 10000000"). The demos open on the secondary monitor and
# never take focus (FORGE_MONITOR); each prints the f32 spacing of a position at its scene's far
# edge, kept in the summary.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:?usage: tools/origins.sh OUT [BIN] [ORIGINS]}
bin=${2:-$root/target/release}
origins=${3:-"10000 100000 1000000 10000000"}
mkdir -p "$out/logs"
summary=$out/summary.txt
exe=""
[ -f "$bin/asteroids.exe" ] && exe=.exe
asteroids=$bin/asteroids$exe
city=$bin/city-blocks$exe
imgdiff=$root/target/release/imgdiff$exe
for tool in "$asteroids" "$city" "$imgdiff"; do
  [ -f "$tool" ] || { echo "missing $tool: build with cargo build --release" >&2; exit 1; }
done
cd "$root"
echo "tools/origins.sh $out from $bin, offsets $origins, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD 2>/dev/null)" | tee "$summary"

status=0
keys="forge_app: gpu:|ERROR|Error|panicked|selected GPU|checksum|world's origin"
# capture NAME FRAME DEMO ARGS...: the frame FRAME of DEMO to OUT/NAME.png, its log to
# OUT/logs/NAME.log, its key lines to the summary.
capture() {
  local name=$1 frame=$2 demo=$3
  shift 3
  local log=$out/logs/$name.log start=$SECONDS
  "$demo" --frames $((frame + 1)) --capture "$out/$name.png" --capture-frame "$frame" "$@" 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g' > "$log"
  if [ -f "$out/$name.png" ]; then
    echo "$name: captured in $((SECONDS - start)) s"
    echo "== $name: captured in $((SECONDS - start)) s ($(basename "$demo") $*)" >> "$summary"
  else
    echo "$name: NO CAPTURE after $((SECONDS - start)) s, see $log"
    echo "== $name: NO CAPTURE ($(basename "$demo") $*)" >> "$summary"
    status=1
  fi
  grep -E "$keys" "$log" | grep -v "GOG" | cut -c1-300 >> "$summary"
  grep -E "ERROR|panicked" "$log" | head -n 5
}
# pair BASE NEW LABEL: the pixels that differ, the largest channel error and, for a difference,
# LDR-FLIP's mean and largest value; the difference and the error map beside NEW.
pair() {
  [ -f "$1" ] && [ -f "$2" ] || return 0
  local stem=${2%.png} result line flip count max mean peak
  result=$("$imgdiff" "$1" "$2" --out "$stem-diff.png" --flip-map "$stem-flip.png" 2>&1)
  line=$(grep "pixels differ" <<< "$result" | tail -n 1)
  flip=$(grep "LDR-FLIP" <<< "$result" | tail -n 1)
  count=$(sed -n 's/.*: \([0-9]*\) \/ [0-9]* pixels differ.*/\1/p' <<< "$line")
  max=$(sed -n 's/.*max channel error \([0-9]*\).*/\1/p' <<< "$line")
  mean=$(sed -n 's/.*: mean \([0-9.]*\),.*/\1/p' <<< "$flip")
  peak=$(sed -n 's/.* max \([0-9.]*\) at .*/\1/p' <<< "$flip")
  if [ -z "$count" ]; then
    echo "$3: imgdiff failed" | tee -a "$summary"
    status=1
  elif [ "$count" != 0 ]; then
    echo "$3: $count px differ (max $max), FLIP mean $mean, max $peak" | tee -a "$summary"
  else
    echo "$3: 0 px" | tee -a "$summary"
  fi
}

# The same views as the batch's city60 and ast240 (tools/captures.sh), every page resident, no
# TAA on the ballad: the raw frame shows where the geometry moved.
for origin in 0 $origins; do
  capture "city-o$origin" 60 "$city" --stream-pool 0 --origin "$origin"
  capture "ast-o$origin" 240 "$asteroids" --fixed-step --no-taa --origin "$origin"
done
echo "== against the same view at the origin" | tee -a "$summary"
for origin in $origins; do
  pair "$out/city-o0.png" "$out/city-o$origin.png" "city at $origin m"
  pair "$out/ast-o0.png" "$out/ast-o$origin.png" "ballad at $origin m"
done
echo "captures, differences and error maps in $out; logs in $out/logs, summary in $summary" | tee -a "$summary"
exit $status
