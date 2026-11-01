#!/usr/bin/env bash
# pbrt-v4's reference renders for D-083's calibration, the third reference beside Falcor's
# (tools/gi-compare.sh's PBRT_OUT): a scene through tools/pbrt-variant.py, then pbrt-v4 on the CPU.
#
#   tools/pbrt-render.sh NAME IN.pbrt [--clay] [--flat] [--opaque]
#
# Writes PBRT_OUT/NAME.exr (linear, the scene's own units), NAME.log and the variant's scene in
# PBRT_OUT/NAME/. The flags are tools/pbrt-variant.py's: --flat and --opaque make pbrt-v4 render the
# geometry and materials the converter's glTF gives Forge and Falcor (the faces' own normals, no
# diffuse transmission), --clay every material a 50 % grey diffuse.
# - PBRT: pbrt-v4's executable (default ../pbrt-v4/build/Release/pbrt.exe beside the checkout).
# - PBRT_OUT: the renders' folder (default ../pbrt-out beside the checkout).
# - PBRT_SPP: samples a pixel (default 1,024: about 12 minutes a room at 1280 x 720 on 16 threads).
# - PBRT_THREADS: pbrt-v4's threads (default all but four, so the other sessions keep some).
#
# pbrt-v4 runs on the CPU: no GPU lock. Its scenes stay outside the repository (scenes-ref/,
# D-083), as its renders.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
name=${1:?usage: tools/pbrt-render.sh NAME IN.pbrt [--clay] [--flat] [--opaque]}
scene=${2:?usage: tools/pbrt-render.sh NAME IN.pbrt [--clay] [--flat] [--opaque]}
shift 2
pbrt=${PBRT:-$(dirname "$root")/pbrt-v4/build/Release/pbrt.exe}
out=${PBRT_OUT:-$(dirname "$root")/pbrt-out}
spp=${PBRT_SPP:-1024}
cores=$(nproc 2> /dev/null || echo 8)
threads=${PBRT_THREADS:-$((cores > 8 ? cores - 4 : cores))}
[ -x "$pbrt" ] || { echo "no pbrt-v4 at $pbrt: build it first" >&2; exit 2; }
win() { if command -v cygpath > /dev/null; then cygpath -w "$1"; else echo "$1"; fi; }
python=$(command -v python3 || command -v python)
mkdir -p "$out"
"$python" -I "$root/tools/pbrt-variant.py" "$scene" "$out/$name" "$@" || exit 1
start=$SECONDS
# pbrt-v4 prints its progress bar on one line, rewritten: the log keeps its last state.
"$pbrt" --spp "$spp" --nthreads "$threads" --outfile "$(win "$out/$name.exr")" "$(win "$out/$name/scene.pbrt")" 2>&1 |
  tr -s '\r\n' '\n' | grep -v '^Rendering: \[' > "$out/$name.log"
status=${PIPESTATUS[0]}
echo "$name: pbrt-v4 exit $status, $spp samples a pixel on $threads threads in $((SECONDS - start)) s" | tee -a "$out/$name.log"
exit "$status"
