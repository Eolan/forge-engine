#!/usr/bin/env bash
# Writes the capture batch every rendering change is checked with (issue #74; docs/PROCESS.md):
# 28 fixed-step captures of meshlets, the ballad, city-blocks and its island (#96), on the mesh
# path and on the fallback (`--force-fallback`). Compare two batches with tools/compare.sh.
#
#   tools/captures.sh OUT [BIN]
#
# OUT: the directory to write (created). BIN: the directory holding the demo binaries (default
# target/release; a baseline built in another tree, see docs/PROCESS.md). The demos open on the
# secondary monitor and never take focus (FORGE_MONITOR). The batch takes a few minutes; each
# capture prints a line as it lands, and a demo's errors and panics. With FORGE_KEEP_LOGS=1 each
# run's full log is kept in OUT/logs/ and its key lines in OUT/summary.txt, for a cloud session
# to read (tools/report.sh gathers them); a local session needs neither.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:?usage: tools/captures.sh OUT [BIN]}
bin=${2:-$root/target/release}
keep=${FORGE_KEEP_LOGS:-0}
mkdir -p "$out"
summary=$out/summary.txt
exe=""
[ -f "$bin/asteroids.exe" ] && exe=.exe
meshlets=$bin/meshlets$exe
asteroids=$bin/asteroids$exe
city=$bin/city-blocks$exe
for demo in "$meshlets" "$asteroids" "$city"; do
  [ -f "$demo" ] || { echo "missing $demo: build with cargo build --release" >&2; exit 1; }
done
cd "$root"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
header="tools/captures.sh $out from $bin, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD 2>/dev/null)"
echo "$header"
if [ "$keep" != 0 ]; then
  mkdir -p "$out/logs"
  echo "$header" > "$summary"
fi

status=0
# Lines of a run's log worth keeping in the summary.
keys="forge_app: gpu:|ERROR|Error|panicked|selected GPU|checksum|world's origin"
# capture NAME FRAME DEMO ARGS...: the frame FRAME of DEMO to OUT/NAME.png; its errors printed,
# its log kept with FORGE_KEEP_LOGS=1.
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
  grep -E "ERROR|panicked" "$log" | head -n 5
  if [ "$keep" != 0 ]; then
    cp "$log" "$out/logs/$name.log"
    echo "== $name: $verdict ($(basename "$demo") $*)" >> "$summary"
    grep -E "$keys" "$log" | grep -v "GOG" | cut -c1-300 >> "$summary"
  fi
}

for path in mesh fb; do
  flag=()
  [ "$path" = fb ] && flag=(--force-fallback)
  # meshlets: the static view, the orbit, and the orbit without LOD or occlusion.
  capture "$path-static60" 60 "$meshlets" "${flag[@]}"
  capture "$path-orbit120" 120 "$meshlets" --orbit "${flag[@]}"
  capture "$path-nolod120" 120 "$meshlets" --orbit --no-lod "${flag[@]}"
  capture "$path-noocc120" 120 "$meshlets" --orbit --no-occlusion "${flag[@]}"
  # The ballad at fixed steps: frame 600 with and without TAA, frame 240 for the A/B harness.
  capture "$path-ast-notaa600" 600 "$asteroids" --fixed-step --no-taa "${flag[@]}"
  capture "$path-ast-taa600" 600 "$asteroids" --fixed-step "${flag[@]}"
  capture "$path-ast240" 240 "$asteroids" --fixed-step --no-taa "${flag[@]}"
  capture "$path-ast240-noocc" 240 "$asteroids" --fixed-step --no-taa --no-occlusion "${flag[@]}"
  capture "$path-ast240-nocone" 240 "$asteroids" --fixed-step --no-taa --no-cone "${flag[@]}"
  capture "$path-ast240-culled" 240 "$asteroids" --fixed-step --no-taa --show-culled "${flag[@]}"
  # city-blocks: every page resident (streaming would make the start depend on timing), and
  # the gallery of the twenty props.
  capture "$path-city60" 60 "$city" --stream-pool 0 "${flag[@]}"
  capture "$path-cityorbit120" 120 "$city" --stream-pool 0 --orbit "${flag[@]}"
  capture "$path-gallery60" 60 "$city" --gallery "${flag[@]}"
  # The island (#96) from its first view on the coast: its heightfield, rocks and sea.
  capture "$path-island60" 60 "$city" --island 7 --stream-pool 0 "${flag[@]}"
done
closing="captures in $out: $(ls "$out"/*.png 2>/dev/null | wc -l) images"
[ "$keep" != 0 ] && closing="$closing; logs in $out/logs, summary in $summary" && echo "$closing" >> "$summary"
echo "$closing"
exit $status
