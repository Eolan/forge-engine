#!/usr/bin/env bash
# The first map's check (D-062, #266): the village on its own plan at 600 m, one seed, five
# fixed views and one walked path, shot under the GPU lock, laid out on a contact sheet and
# each compared with the last accepted set by LDR-ꟻLIP (imgdiff). About two minutes: the
# check per commit for the map, where the tiers run at milestones on the owner's word; the
# gate (tools/verify.sh --tier gate) still runs on every push.
#
#   tools/map-check.sh            shoot, compare with captures/map/accepted, keep the run
#   tools/map-check.sh --accept   the same, then make this run the accepted set
#
# The views come from the demo's own log (`city-plan --village` names them for `--view`), so
# they follow the seed. Run from anywhere; the runs and their logs stay in captures/map/<time>/.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
accept=0
[ "${1:-}" = "--accept" ] && accept=1
if [ -n "${FORGE_GPU_LOCK:-}" ]; then
  lock=$FORGE_GPU_LOCK
elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-gpu.lock
else
  lock=${TMPDIR:-/tmp}/forge-gpu.lock
fi
stamp=$(date +%Y%m%d-%H%M%S)
out=$root/captures/map/$stamp
accepted=$root/captures/map/accepted
mkdir -p "$out"
size=600

echo "== building"
(cd "$root" && cargo build --release -q -p city-plan -p imgdiff -p contact-sheet) || exit 1

echo "== waiting for the GPU lock ($lock)"
until mkdir "$lock" 2> /dev/null; do sleep 5; done
echo "forge map-check.sh" > "$lock/owner"
trap 'rm -f "$lock/owner"; rmdir "$lock" 2> /dev/null' EXIT

shots=()
cd "$root" || exit 1
# A shot of `name` with the demo's arguments after it: `frames` (240) at a fixed step, the
# capture at `frame` (200).
shoot() {
  local name=$1
  shift
  FORGE_MONITOR=${FORGE_MONITOR:-secondary} timeout 300 target/release/city-plan.exe \
    --village "$size" --time-of-day 0.75 --fixed-step --frames "${frames:-240}" \
    --capture "$out/$name.png" --capture-frame "${frame:-200}" "$@" > "$out/$name.log" 2>&1
  local status=$?
  local gpu
  gpu=$(sed 's/\x1b\[[0-9;]*m//g' "$out/$name.log" | grep -oE 'gpu: [0-9.]+ ms' | head -1)
  echo "$name: exit $status, $gpu"
  shots+=("$out/$name.png")
}
# From the air first: its log names the other views.
shoot air
named() {
  sed 's/\x1b\[[0-9;]*m//g' "$out/air.log" | grep -oE "the village's $1: [^;]*" | head -1 | sed "s/the village's $1: //"
}
for name in house green main junction; do
  view=$(named "$name")
  if [ -z "$view" ]; then
    echo "$name: the demo's log names no such view"
    continue
  fi
  # shellcheck disable=SC2086
  shoot "$name" $view
done
walk=$(named walk)
if [ -n "$walk" ]; then
  # shellcheck disable=SC2086
  frames=300 frame=290 shoot walk $walk --walk-start
fi
rm -f "$lock/owner"; rmdir "$lock" 2> /dev/null; trap - EXIT

target/release/contact-sheet "$out/sheet.png" "${shots[@]}" --columns 3 --width 600 > /dev/null
echo "== contact sheet: $out/sheet.png"

if [ -d "$accepted" ]; then
  echo "== against the accepted set"
  for shot in "${shots[@]}"; do
    name=$(basename "$shot")
    if [ -f "$accepted/$name" ]; then
      result=$(target/release/imgdiff "$accepted/$name" "$shot" 2>&1)
      flip=$(echo "$result" | grep -oE 'mean [0-9.]+' | head -1)
      if echo "$result" | grep -q '^Error'; then
        echo "$name: NOT COMPARED: $(echo "$result" | grep '^Error' | head -1)"
      else
        echo "$name: ${flip:-identical}"
      fi
    else
      echo "$name: new"
    fi
  done
fi
if [ $accept -eq 1 ]; then
  rm -rf "$accepted"
  cp -r "$out" "$accepted"
  echo "== accepted: $accepted"
fi
