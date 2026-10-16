#!/usr/bin/env bash
# Load times (#274): runs a demo of this tree's release build a few times under the GPU lock and
# prints, for each run, the wall time from the process start to its first log line (the window
# opening) and to its `loaded` line (forge_app::loading: the loading screen gone), then the best.
#
#   tools/load-time.sh city-plan --village 600          three warm runs
#   tools/load-time.sh --runs 5 city-plan               five
#   tools/load-time.sh --fresh city-plan --village 600  each on a fresh cache
#
# `--fresh` sets the tree's cache/ aside for each run and puts it back after (the run's own cache
# is removed): the shaders compile and the meshes cook as on a first start, and the island's
# products wait where they were. The machine is shared: compare the best of three, and a base
# build's runs taken in turn with the change's. The runs' logs stay in captures/load-time/<time>/.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
runs=3
fresh=0
while [ $# -gt 0 ]; do
  case $1 in
    --runs) runs=$2; shift 2 ;;
    --fresh) fresh=1; shift ;;
    *) break ;;
  esac
done
[ $# -ge 1 ] || { echo "usage: tools/load-time.sh [--runs N] [--fresh] DEMO [ARGS...]"; exit 2; }
demo=$1
shift
exe=$root/target/release/$demo
[ -x "$exe.exe" ] && exe=$exe.exe
[ -x "$exe" ] || { echo "no release build of $demo: cargo build --release -p $demo"; exit 2; }
args=("$@")
# A few frames after the load; `--frames` cannot be given twice.
[[ " ${args[*]} " == *" --frames "* ]] || args+=(--frames 5)
if [ -n "${FORGE_GPU_LOCK:-}" ]; then
  lock=$FORGE_GPU_LOCK
elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-gpu.lock
else
  lock=${TMPDIR:-/tmp}/forge-gpu.lock
fi
out=$root/captures/load-time/$(date +%Y%m%d-%H%M%S)
mkdir -p "$out"
aside=$root/cache.load-time-aside
restore() {
  if [ -d "$aside" ]; then
    rm -rf "$root/cache"
    mv "$aside" "$root/cache"
  fi
  rm -f "$lock/owner"
  rmdir "$lock" 2> /dev/null
}

echo "== waiting for the GPU lock ($lock)"
deadline=$(($(date +%s) + 3600))
until mkdir "$lock" 2> /dev/null; do
  [ "$(date +%s)" -gt "$deadline" ] && { echo "the GPU lock stayed taken an hour"; exit 3; }
  sleep 5
done
echo "forge load-time.sh" > "$lock/owner"
trap restore EXIT

best=
cd "$root" || exit 1
for k in $(seq 1 "$runs"); do
  [ $fresh -eq 1 ] && [ -d "$root/cache" ] && mv "$root/cache" "$aside"
  log=$out/run$k.log
  start=$(date +%s%N)
  FORGE_MONITOR=${FORGE_MONITOR:-secondary} timeout 900 "$exe" "${args[@]}" > "$log" 2>&1
  status=$?
  sed -i 's/\x1b\[[0-9;]*m//g' "$log"
  first=$(grep -m1 -E '^[0-9]{4}-[0-9]{2}-[0-9]{2}T' "$log" | awk '{print $1}')
  loaded=$(grep -m1 'forge_app::loading: loaded' "$log" | awk '{print $1}')
  if [ -z "$loaded" ]; then
    echo "run $k: exit $status, no 'loaded' line (see $log)"
  else
    window=$((($(date -d "$first" +%s%N) - start) / 1000000))
    ms=$((($(date -d "$loaded" +%s%N) - start) / 1000000))
    echo "run $k: window $window ms, loaded $ms ms"
    { [ -z "$best" ] || [ "$ms" -lt "$best" ]; } && best=$ms
  fi
  if [ $fresh -eq 1 ] && [ -d "$aside" ]; then
    rm -rf "$root/cache"
    mv "$aside" "$root/cache"
  fi
done
note=
[ $fresh -eq 1 ] && note=", fresh cache"
[ -n "$best" ] && echo "== best: loaded $best ms ($demo ${args[*]}$note)"
echo "== logs: $out"
