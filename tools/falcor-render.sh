#!/usr/bin/env bash
# Falcor's reference renders for D-083's calibration: tools/falcor-render.py in Mogwai, headless,
# under the GPU lock, with Lambert's diffuse (D-086: every comparison runs Falcor with its diffuse
# switched to Lambert; Falcor's default is Frostbite's) unless FALCOR_DIFFUSE=frostbite.
#
#   tools/falcor-render.sh NAME [VAR=VALUE...]
#
# NAME: the files' base name in FALCOR_OUT (default ../falcor/out beside the checkout):
# NAME.AccumulatePass.output.0.exr (linear, tools/gi-compare.sh's references),
# NAME.ToneMapper.dst.0.png and the log, NAME.out. VAR=VALUE: tools/falcor-render.py's settings
# (BISTRO_SCENE, BISTRO_VIEW, BISTRO_SPP, ...). FALCOR: Falcor's checkout (default
# ../falcor/Falcor beside this one), built (build/windows-ninja-msvc/bin/Release/Mogwai.exe).
#
# The diffuse model is a define of Falcor's shaders (Rendering/Materials/BSDFConfig.slangh's
# DiffuseBrdf), read at run time from the build's shaders folder: set for the render and put
# back after it, also when it fails or is interrupted. Falcor runs unmodified otherwise.
#
# The Cornell box (tools/cornell-box.py, then tools/pbrt-to-gltf.py into assets/external/cornell):
#   tools/falcor-render.sh cornell-lam16 BISTRO_SCENE='D:\...\assets\external\cornell\cornell.pyscene' \
#     BISTRO_VIEW=0,2.8,12,0,0 BISTRO_FOV=37.8493 BISTRO_SIZE=1600,900 BISTRO_SPP=4096 \
#     BISTRO_BOUNCES=16 BISTRO_LIGHTS=emissive BISTRO_DIFFUSE=1 BISTRO_EMITTER_STRENGTH=85
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
name=${1:?usage: tools/falcor-render.sh NAME [VAR=VALUE...]}
shift
falcor=${FALCOR:-$(dirname "$root")/falcor/Falcor}
bin=$falcor/build/windows-ninja-msvc/bin/Release
out=${FALCOR_OUT:-$(dirname "$root")/falcor/out}
config=$bin/shaders/Rendering/Materials/BSDFConfig.slangh
[ -f "$bin/Mogwai.exe" ] || { echo "no Mogwai in $bin: build Falcor first" >&2; exit 2; }
[ -f "$config" ] || { echo "no $config" >&2; exit 2; }
case ${FALCOR_DIFFUSE:-lambert} in
  lambert) model=DiffuseBrdfLambert ;;
  frostbite) model=DiffuseBrdfFrostbite ;;
  *) echo "FALCOR_DIFFUSE: lambert or frostbite" >&2; exit 2 ;;
esac
win() { if command -v cygpath > /dev/null; then cygpath -w "$1"; else echo "$1"; fi; }
if [ -n "${FORGE_GPU_LOCK:-}" ]; then
  lock=$FORGE_GPU_LOCK
elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-gpu.lock
else
  lock=${TMPDIR:-/tmp}/forge-gpu.lock
fi
mkdir -p "$out"
saved=$(mktemp)
cp "$config" "$saved"
locked=0
cleanup() {
  cp "$saved" "$config" && rm -f "$saved"
  [ $locked = 1 ] && rm -rf "$lock"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
until mkdir "$lock" 2> /dev/null; do
  echo "the GPU is busy ($(cat "$lock/owner" 2> /dev/null || echo "no owner file")): retrying in 30 s"
  sleep 30
done
echo "tools/falcor-render.sh $name, pid $$, since $(date -u +%FT%TZ)" > "$lock/owner"
locked=1
sed -i "s/^#define DiffuseBrdf DiffuseBrdf[A-Za-z]*$/#define DiffuseBrdf $model/" "$config"
grep -q "^#define DiffuseBrdf $model$" "$config" || { echo "the diffuse define is not where it was in $config" >&2; exit 2; }
env "$@" BISTRO_OUT="$(win "$out")" BISTRO_NAME="$name" timeout 3600 "$bin/Mogwai.exe" --headless \
  --script "$(win "$root/tools/falcor-render.py")" > "$out/$name.out" 2>&1
code=$?
grep -E "^(EMISSIVE|MATERIAL|LIGHTS|CAMERA|RENDERED)" "$out/$name.out"
echo "$name: exit $code, $out/$name.AccumulatePass.output.0.exr"
exit $code
