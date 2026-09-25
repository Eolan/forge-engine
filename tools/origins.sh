#!/usr/bin/env bash
# The far-origin check (issue #93; docs/PROCESS.md): the city's south view and the ballad's fixed
# step captured with the scene moved 10^4 to 10^7 m from the world's origin (`--origin`), each
# compared with the same view at the origin in pixels and in LDR-FLIP (issue #75), with the
# difference and the error map written beside it. The scene, its camera and everything anchored
# to them move together: with the instance table in integer cells (D-004's amendment) every
# offset gives the origin's image, up to a few pixels from the split's 0.1 mm rounding; with the
# world-space f32 table before it, the differences were the measurement. The exit code is 0
# unless a capture or a comparison failed to run.
#
#   tools/origins.sh OUT [BIN] [ORIGINS]
#
# OUT: the directory to write (created). BIN: the directory holding the demo binaries (default
# target/release). ORIGINS: the offsets in metres, quoted (default "10000 100000 1000000
# 10000000"). The demos open on the secondary monitor and never take focus (FORGE_MONITOR); each
# prints the f32 spacing of a position at its scene's far edge. With FORGE_KEEP_LOGS=1 each
# run's full log is kept in OUT/logs/ and the lines printed in OUT/summary.txt, for a cloud
# session to read (tools/report.sh gathers them).
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:?usage: tools/origins.sh OUT [BIN] [ORIGINS]}
bin=${2:-$root/target/release}
origins=${3:-"10000 100000 1000000 10000000"}
keep=${FORGE_KEEP_LOGS:-0}
mkdir -p "$out"
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
log=$(mktemp)
trap 'rm -f "$log"' EXIT
header="tools/origins.sh $out from $bin, offsets $origins, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD 2>/dev/null)"
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

status=0
keys="forge_app: gpu:|ERROR|Error|panicked|selected GPU|checksum|world's origin"
# capture NAME FRAME DEMO ARGS...: the frame FRAME of DEMO to OUT/NAME.png.
capture() {
  local name=$1 frame=$2 demo=$3
  shift 3
  local start=$SECONDS
  "$demo" --frames $((frame + 1)) --capture "$out/$name.png" --capture-frame "$frame" "$@" 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g' > "$log"
  local verdict="captured in $((SECONDS - start)) s"
  if [ ! -f "$out/$name.png" ]; then
    verdict="NO CAPTURE after $((SECONDS - start)) s"
    status=1
  fi
  echo "$name: $verdict"
  grep -E "ERROR|panicked|world's origin" "$log" | cut -c1-200 | head -n 5
  if [ "$keep" != 0 ]; then
    cp "$log" "$out/logs/$name.log"
    echo "== $name: $verdict ($(basename "$demo") $*)" >> "$summary"
    grep -E "$keys" "$log" | grep -v "GOG" | cut -c1-300 >> "$summary"
  fi
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
    say "$3: imgdiff failed"
    status=1
  elif [ "$count" != 0 ]; then
    say "$3: $count px differ (max $max), FLIP mean $mean, max $peak"
  else
    say "$3: 0 px"
  fi
}

# The same views as the batch's city60 and ast240 (tools/captures.sh), every page resident, no
# TAA on the ballad: the raw frame shows where the geometry moved.
for origin in 0 $origins; do
  capture "city-o$origin" 60 "$city" --stream-pool 0 --origin "$origin"
  capture "ast-o$origin" 240 "$asteroids" --fixed-step --no-taa --origin "$origin"
done
say "== against the same view at the origin"
for origin in $origins; do
  pair "$out/city-o0.png" "$out/city-o$origin.png" "city at $origin m"
  pair "$out/ast-o0.png" "$out/ast-o$origin.png" "ballad at $origin m"
done
closing="captures, differences and error maps in $out"
[ "$keep" != 0 ] && closing="$closing; logs in $out/logs, summary in $summary"
say "$closing"
exit $status
