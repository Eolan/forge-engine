#!/usr/bin/env bash
# Writes the capture batch every rendering change is checked with (issue #74; docs/PROCESS.md):
# 26 fixed-step captures of meshlets, the ballad and city-blocks, on the mesh path and on the
# fallback (`--force-fallback`). Compare two batches with tools/compare.sh.
#
#   tools/captures.sh OUT [BIN]
#
# OUT: the directory to write (created). BIN: the directory holding the demo binaries
# (default target/release; a baseline built in another tree, see docs/PROCESS.md). The demos
# open on the secondary monitor and never take focus (FORGE_MONITOR). Errors and panics are
# printed; the batch takes a few minutes.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
out=${1:?usage: tools/captures.sh OUT [BIN]}
bin=${2:-$root/target/release}
mkdir -p "$out"
exe=""
[ -f "$bin/asteroids.exe" ] && exe=.exe
meshlets=$bin/meshlets$exe
asteroids=$bin/asteroids$exe
city=$bin/city-blocks$exe
for demo in "$meshlets" "$asteroids" "$city"; do
  [ -f "$demo" ] || { echo "missing $demo: build with cargo build --release" >&2; exit 1; }
done
cd "$root"

# Runs a demo, printing only its errors and panics (colour codes stripped).
run() { "$@" 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E "ERROR|Error|panicked" || true; }
# capture NAME FRAME DEMO ARGS...: the frame FRAME of DEMO to OUT/NAME.png.
capture() {
  local name=$1 frame=$2 demo=$3
  shift 3
  run "$demo" --frames $((frame + 1)) --capture "$out/$name.png" --capture-frame "$frame" "$@"
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
echo "captures in $out: $(ls "$out"/*.png 2>/dev/null | wc -l) images"
