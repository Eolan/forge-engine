#!/usr/bin/env bash
# The occasional AMD check (#28; docs/PROCESS.md, "The AMD check"): the dev machine's AMD iGPU
# (RDNA 2, `FORGE_GPU=amd`) runs meshlets, asteroids and the physics lab's yard on the mesh path
# and the fallback, each twice, then each under sync validation. A different vendor's driver
# finds bugs NVIDIA's lets through (#28's hang, #207's barriers). Not a tier and not a gate:
# run it on a night with a Tier 2, after it, and report what it prints.
#
#   tools/amd-check.sh [OUT]
#
# OUT: the captures and logs (default captures/amd/<time>-<sha>). One run at a time, and the
# check STOPS at the first lost device: repeated losses once disabled both GPUs and crashed the
# iGPU. Look at that run's log before running anything else on the iGPU. It prints, per demo:
# whether reruns are identical and the mesh path matches the fallback (0 pixels both), and the
# validation messages.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
sha=$(git rev-parse --short HEAD 2>/dev/null || echo nogit)
out=${1:-$root/captures/amd/$(date +%Y%m%d-%H%M%S)-$sha}
bin=$root/target/release
if [ -n "${FORGE_GPU_LOCK:-}" ]; then
  lock=$FORGE_GPU_LOCK
elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-gpu.lock
else
  lock=${TMPDIR:-/tmp}/forge-gpu.lock
fi
mkdir -p "$out"

# The runs: NAME|FRAME|COMMAND, captured at FRAME.
runs=(
  "meshlets|120|$bin/meshlets --orbit"
  "asteroids|600|$bin/asteroids --fixed-step"
  "yard|120|$bin/physics-lab --lab yard --fixed-step"
)

for _ in $(seq 1 120); do
  mkdir "$lock" 2> /dev/null && break
  sleep 30
done
[ -d "$lock" ] && [ ! -f "$lock/owner" ] || { echo "the GPU lock stayed taken" >&2; exit 1; }
echo "amd-check $$" > "$lock/owner"
release() { rm -f "$lock/owner"; rmdir "$lock" 2> /dev/null; }
trap release EXIT

# one TAG CMD...: one run on the iGPU; stops the check at a lost device or a failure.
one() {
  local tag=$1
  shift
  FORGE_MONITOR=${FORGE_MONITOR:-secondary} FORGE_GPU=amd timeout 600 "$@" 2>&1 |
    sed 's/\x1b\[[0-9;]*m//g' > "$out/$tag.log"
  local code=${PIPESTATUS[0]}
  echo "$tag: exit $code, $(grep -oE 'exited cleanly frames=[0-9]+' "$out/$tag.log")"
  if grep -q 'device has been lost' "$out/$tag.log"; then
    echo "STOPPED: $tag lost the device; read $out/$tag.log before the next iGPU run"
    exit 1
  fi
  [ "$code" = 0 ] || { echo "STOPPED: $tag failed"; exit 1; }
}

for r in "${runs[@]}"; do
  IFS='|' read -r name frame cmd <<< "$r"
  for path in mesh fb; do
    flag=()
    [ $path = fb ] && flag=(--force-fallback)
    for n in 1 2; do
      # shellcheck disable=SC2086 # the command's words
      one "$name-$path-$n" $cmd "${flag[@]}" --frames $((frame + 1)) \
        --capture "$out/$name-$path-$n.png" --capture-frame "$frame"
    done
    # shellcheck disable=SC2086
    FORGE_SYNC_VALIDATION=1 one "$name-$path-validate" $cmd "${flag[@]}" --frames 60 --validate
    echo "  validation: $(grep -E ' (WARN|ERROR) ' "$out/$name-$path-validate.log" |
      grep -vc 'GalaxyOverlay') messages"
  done
done
release
trap - EXIT

for r in "${runs[@]}"; do
  IFS='|' read -r name _ _ <<< "$r"
  for pair in "mesh-1 mesh-2" "fb-1 fb-2" "fb-1 mesh-1"; do
    read -r a b <<< "$pair"
    echo "$name, $a vs $b: $("$bin/imgdiff" "$out/$name-$a.png" "$out/$name-$b.png" |
      head -1 | grep -oE '[0-9]+ / [0-9]+ pixels differ')"
  done
done | tee "$out/summary.txt"
