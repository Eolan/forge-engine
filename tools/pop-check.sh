#!/usr/bin/env bash
# The village's walk-in at noon checked for pops (#363): from the start view at 8 m/s for 128 m,
# a capture every third frame, then `pop-steps` over the middle of the view (the screen's edges
# and the near ground left out, where near objects sweep past; past 128 m a tree near the camera
# does). A pop is a 16-pixel tile whose brightness steps by 5 sRGB levels or more in one capture
# while the captures around it change less: a tree's impostor handing over to its shells in one
# frame scored 5–12 before 16c06454, none after. About a minute.
#
#   tools/pop-check.sh [BIN]
#
# BIN: the directory holding city-plan and pop-steps (default target/release, built first). The
# run stays in captures/pops/<time>/: its log, `pop-steps`' list (pops.txt) and, when it fails,
# its frames. More than FORGE_POPS_MAX pops (default 2) fails it (exit 1). The night check runs it
# after the batch (tools/night.sh).
export FORGE_CFG=none
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-}
max=${FORGE_POPS_MAX:-2}
if [ -n "${FORGE_GPU_LOCK:-}" ]; then
  lock=$FORGE_GPU_LOCK
elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-gpu.lock
else
  lock=${TMPDIR:-/tmp}/forge-gpu.lock
fi
out=$root/captures/pops/$(date +%Y%m%d-%H%M%S)
mkdir -p "$out"
if [ -z "$bin" ]; then
  bin=$root/target/release
  (cd "$root" && cargo build --release -q -p city-plan -p pop-steps) || exit 2
fi
exe() { if [ -f "$bin/$1.exe" ]; then echo "$bin/$1.exe"; else echo "$bin/$1"; fi; }

echo "== waiting for the GPU lock ($lock)"
until mkdir "$lock" 2> /dev/null; do sleep 5; done
echo "forge pop-check.sh" > "$lock/owner"
trap 'rm -f "$lock/owner"; rmdir "$lock" 2> /dev/null' EXIT
cd "$root" || exit 2
FORGE_SOUND=0 FORGE_MONITOR=${FORGE_MONITOR:-secondary} timeout 600 "$(exe city-plan)" \
  --village 600 --time-of-day 0.5 --fixed-step --dolly 8 --frames 960 --no-fauna \
  --capture "$out/f.png" --capture-every 3 > "$out/run.log" 2>&1
status=$?
rm -f "$lock/owner"; rmdir "$lock" 2> /dev/null; trap - EXIT
if [ $status != 0 ]; then
  echo "pop-check: the walk-in failed (exit $status), its log: $out/run.log"
  exit 2
fi

"$(exe pop-steps)" "$out" --prefix f --region 192,0,1216,680 --from 66 --min 5 --top 10 > "$out/pops.txt" 2>&1
pops=$(tail -1 "$out/pops.txt" | awk '{print $1}')
tail -12 "$out/pops.txt"
if ! [ "$pops" -ge 0 ] 2> /dev/null; then
  echo "pop-check: pop-steps gave no count, see $out/pops.txt"
  exit 2
fi
if [ "$pops" -le "$max" ]; then
  rm -f "$out"/f-*.png "$out/f.png"
  echo "pop-check: $pops pops of 5 levels or more on the walk-in (at most $max), passed"
  exit 0
fi
echo "pop-check: $pops pops of 5 levels or more on the walk-in (at most $max), FAILED; frames in $out"
exit 1
