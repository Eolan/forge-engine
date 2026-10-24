#!/usr/bin/env bash
# The light check (the audit's step 2, 2026-10-25; docs/PROCESS.md, "The light check"): the
# village's named views at a few hours of the day, shot as HDR captures (their PQ codes), and
# the mean log2 luminance of named patches in each compared with a reference set, in EV: a
# lighting change is then judged where it landed and by how much, patch by patch, instead of by
# a whole frame's FLIP. Beside it, the still-camera measure per view: the pixels that differ
# between frames 200 and 210 with the camera still, which a new source of noise raises.
#
#   tools/light-check.sh              shoot, compare with captures/light/reference, keep the run
#   tools/light-check.sh --reference  shoot with probes twice as dense, make the run the reference
#   tools/light-check.sh --dry-run    print what would run, shoot nothing
#
# The reference is the same views with the probes twice as dense (the cascades 0.5 m apart, the
# room grids 0.75 m, 256 rays: D-078's measure), made once at a milestone; a patch within
# FORGE_LIGHT_TOLERANCE_EV of it passes (0.15 by default). FORGE_LIGHT_HOURS gives the times of
# day (`--time-of-day`: 0.5 noon, 0.75 the afternoon, 0.85 a low sun; default "0.5 0.75 0.85"),
# FORGE_LIGHT_VIEWS the views as the demo's log names them (default "house green hall door room
# landing"), FORGE_LIGHT_STILL_PCT the still-camera limit in percent of the pixels (0.5 by
# default). The patches are tools/light-patches.txt. The runs stay in captures/light/<time>/
# with a table, light.txt. About a minute per view and hour on the RTX 5070 Ti.
# The owner's settings file (forge.cfg, D-069) never changes a scripted run.
export FORGE_CFG=none
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
mode=run
case ${1:-} in
  "") ;;
  --reference) mode=reference ;;
  --dry-run) mode=dry ;;
  *) echo "usage: tools/light-check.sh [--reference | --dry-run]" >&2; exit 2 ;;
esac
hours=${FORGE_LIGHT_HOURS:-0.5 0.75 0.85}
views=${FORGE_LIGHT_VIEWS:-house green hall door room landing}
tolerance=${FORGE_LIGHT_TOLERANCE_EV:-0.15}
still_limit=${FORGE_LIGHT_STILL_PCT:-0.5}
patches=$root/tools/light-patches.txt
reference=$root/captures/light/reference
stamp=$(date +%Y%m%d-%H%M%S)
out=$root/captures/light/$stamp
size=600
# The reference's dense probes (D-078's measure).
dense=(--probe-spacing 0.5 --probe-room-spacing 0.75,0.75 --probe-rays 256 --probe-room-rays 256)

if [ -n "${FORGE_GPU_LOCK:-}" ]; then
  lock=$FORGE_GPU_LOCK
elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-gpu.lock
else
  lock=${TMPDIR:-/tmp}/forge-gpu.lock
fi

# patches_of VIEW: the view's patches as "name x,y,w,h" lines, the `*` lines when it has none.
patches_of() {
  local view=$1 lines
  lines=$(awk -v v="$view" '$1 == v && NF == 6 { print $2, $3 "," $4 "," $5 "," $6 }' "$patches")
  [ -n "$lines" ] || lines=$(awk '$1 == "*" && NF == 6 { print $2, $3 "," $4 "," $5 "," $6 }' "$patches")
  echo "$lines"
}

cd "$root" || exit 1
if [ "$mode" = dry ]; then
  echo "would shoot the views ($views) at the hours ($hours) into $out"
  [ "$mode" = reference ] && echo "with the dense probes: ${dense[*]}"
  for view in $views; do
    echo "$view:"
    patches_of "$view" | sed 's/^/  patch /'
  done
  if [ -d "$reference" ]; then
    echo "then compare with $reference, tolerance $tolerance EV, still camera under $still_limit %"
  else
    echo "no reference yet: run tools/light-check.sh --reference first"
  fi
  exit 0
fi

mkdir -p "$out"
echo "== building"
cargo build --release -q -p city-plan -p imgdiff -p contact-sheet || exit 1
echo "== waiting for the GPU lock ($lock)"
until mkdir "$lock" 2> /dev/null; do sleep 5; done
echo "forge light-check.sh" > "$lock/owner"
trap 'rm -f "$lock/owner"; rmdir "$lock" 2> /dev/null' EXIT

extra=()
[ "$mode" = reference ] && extra=("${dense[@]}")
# shoot NAME HOUR ARGS...: 211 frames at a fixed step, the HDR capture at frame 200 and the
# preview every 10 frames (the still-camera pair, 200 and 210).
shoot() {
  local name=$1 hour=$2
  shift 2
  FORGE_MONITOR=${FORGE_MONITOR:-secondary} timeout 300 target/release/city-plan.exe \
    --village "$size" --time-of-day "$hour" --fixed-step --frames 211 \
    --capture "$out/$name.png" --capture-frame 200 --capture-every 10 --no-fauna \
    --hdr offscreen "$@" "${extra[@]}" > "$out/$name.log" 2>&1
  local status=$?
  echo "$name: exit $status"
}
# From the air first, at the first hour: its log names the views.
first_hour=${hours%% *}
shoot air "$first_hour"
named() {
  sed 's/\x1b\[[0-9;]*m//g' "$out/air.log" | grep -oE "the village's $1: [^;]*" | head -1 | sed "s/the village's $1: //"
}
shots=()
for view in $views; do
  at=$(named "$view")
  if [ -z "$at" ]; then
    echo "$view: the demo's log names no such view"
    continue
  fi
  for hour in $hours; do
    # shellcheck disable=SC2086
    shoot "$view-$hour" "$hour" $at
    shots+=("$out/$view-$hour.png")
  done
done
rm -f "$lock/owner"; rmdir "$lock" 2> /dev/null; trap - EXIT
[ ${#shots[@]} -gt 0 ] && target/release/contact-sheet "$out/sheet.png" "${shots[@]}" --columns 3 --width 600 > /dev/null
echo "== contact sheet: $out/sheet.png"

table=$out/light.txt
status=0
{
  echo "light check $stamp, tolerance $tolerance EV, still camera under $still_limit %"
  printf '%-10s %-5s %-10s %10s %10s %9s  %s\n' view hour patch reference run change verdict
} > "$table"
for view in $views; do
  for hour in $hours; do
    name=$view-$hour
    pq=$out/$name-pq.png
    [ -f "$pq" ] || { echo "$name: no capture" | tee -a "$table"; status=1; continue; }
    # The still camera: frames 200 and 210 of the preview.
    if [ -f "$out/$name-00200.png" ] && [ -f "$out/$name-00210.png" ]; then
      pct=$(target/release/imgdiff "$out/$name-00200.png" "$out/$name-00210.png" --no-flip 2>&1 | grep -oE '[0-9.]+ %' | head -1 | tr -d ' %')
      verdict=ok
      awk -v p="${pct:-0}" -v l="$still_limit" 'BEGIN { exit !(p > l) }' && { verdict=NOISY; status=1; }
      printf '%-10s %-5s %-10s %10s %10s %9s  %s\n' "$view" "$hour" "still" "" "${pct:-?} %" "" "$verdict" >> "$table"
    fi
    if [ ! -d "$reference" ] || [ ! -f "$reference/$name-pq.png" ]; then
      printf '%-10s %-5s %-10s %10s %10s %9s  %s\n' "$view" "$hour" "(patches)" "" "" "" "no reference" >> "$table"
      continue
    fi
    args=()
    names=()
    while read -r patch rect; do
      [ -n "$patch" ] || continue
      args+=(--mean-log-in "$rect")
      names+=("$patch")
    done <<< "$(patches_of "$view")"
    i=0
    while IFS= read -r line; do
      ref=$(echo "$line" | grep -oE 'a [-0-9.]+' | head -1 | cut -c3-)
      run=$(echo "$line" | grep -oE 'b [-0-9.]+' | head -1 | cut -c3-)
      change=$(echo "$line" | grep -oE 'change [-+0-9.]+' | head -1 | cut -c8-)
      verdict=ok
      awk -v c="${change:-0}" -v t="$tolerance" 'BEGIN { exit !(c > t || c < -t) }' && { verdict=FAIL; status=1; }
      printf '%-10s %-5s %-10s %10s %10s %9s  %s\n' "$view" "$hour" "${names[i]:-?}" "${ref:-?}" "${run:-?}" "${change:-?}" "$verdict" >> "$table"
      i=$((i + 1))
    done <<< "$(target/release/imgdiff "$reference/$name-pq.png" "$pq" --no-flip "${args[@]}" 2>&1 | grep 'mean log2 luminance in')"
  done
done
cat "$table"
if [ "$mode" = reference ]; then
  rm -rf "$reference"
  mkdir -p "$(dirname "$reference")"
  cp -r "$out" "$reference"
  echo "== reference: $reference"
  exit 0
fi
[ $status = 0 ] && echo "== the light check passes" || echo "== the light check FAILS (see $table)"
exit $status
