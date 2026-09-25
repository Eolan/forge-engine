#!/usr/bin/env bash
# Writes the capture batch every rendering change is checked with (issue #74; docs/PROCESS.md):
# 26 fixed-step captures of meshlets, the ballad and city-blocks, on the mesh path and on the
# fallback (`--force-fallback`). Compare two batches with tools/compare.sh.
#
#   tools/captures.sh OUT [BIN]
#
# OUT: the directory to write (created): the captures, each run's full log in OUT/logs/ and
# OUT/summary.txt with each capture's outcome and its run's key lines (the GPU time, the
# placement's checksum, errors). A cloud session reads the logs and the summary, not the
# images: tools/report.sh gathers them to commit. BIN: the directory holding the demo binaries
# (default target/release; a baseline built in another tree, see docs/PROCESS.md). The demos
# open on the secondary monitor and never take focus (FORGE_MONITOR). The batch takes a few
# minutes; each capture prints a line as it lands.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:?usage: tools/captures.sh OUT [BIN]}
bin=${2:-$root/target/release}
mkdir -p "$out/logs"
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
echo "tools/captures.sh $out from $bin, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD 2>/dev/null)" | tee "$summary"

status=0
# Lines of a run's log worth keeping in the summary.
keys="forge_app: gpu:|ERROR|Error|panicked|selected GPU|checksum|world's origin"
# capture NAME FRAME DEMO ARGS...: the frame FRAME of DEMO to OUT/NAME.png, its log to
# OUT/logs/NAME.log (colour codes stripped), its key lines to the summary.
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
done
echo "captures in $out: $(ls "$out"/*.png 2>/dev/null | wc -l) images; logs in $out/logs, summary in $summary" | tee -a "$summary"
exit $status
