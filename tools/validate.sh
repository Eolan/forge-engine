#!/usr/bin/env bash
# Runs every demo on both paths under the Vulkan validation layer, synchronization validation
# included (issue #74; docs/PROCESS.md), and prints each run's messages, counted. A clean run
# prints its header line, its duration and its log's length only. The GOG overlay layer's
# naming warnings are noise and dropped. The runs are short on purpose (60–90 frames each, a
# minute or two in all): the layer's cost is in the checks, not the frames.
#
#   tools/validate.sh [BIN] [OUT]
#
# BIN: the demo binaries (default target/release). With FORGE_KEEP_LOGS=1 each run's full log
# is kept in OUT/logs/ (default captures/validate) and what was printed in OUT/summary.txt,
# for a cloud session to read (tools/report.sh gathers them).
#
# FORGE_SETS (issue #134): the runs to make, separated by spaces or commas (default all):
# ballad (with its ships and HDR output), meshlets, city, island (the city's island, its water
# and the island demo), lab (the physics lab), sentinels (the ballad, meshlets and the city's
# first run), all.
# FORGE_PATHS: mesh and fb (default both).
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
bin=${1:-$root/target/release}
out=${2:-$root/captures/validate}
keep=${FORGE_KEEP_LOGS:-0}
sets=" ${FORGE_SETS:-all} "
sets=${sets//,/ }
paths=${FORGE_PATHS:-mesh fb}
paths=${paths//,/ }
for set in $sets; do
  case $set in
    all | sentinels | meshlets | ballad | city | island | lab) ;;
    *) echo "unknown set $set: sentinels, meshlets, ballad, city, island, lab or all" >&2; exit 1 ;;
  esac
done
# sets_of NAME: the sets a run belongs to, from its name without the path.
sets_of() {
  case $1 in
    ballad) echo ballad sentinels ;;
    ships | hdr | hdr-calibration | hdr-switch) echo ballad ;;
    meshlets) echo meshlets sentinels ;;
    hdr-display) echo meshlets ;;
    city) echo city sentinels ;;
    city-* | gallery) echo city ;;
    lab | lab-sea | lab-walk | lab-drive | lab-fly | lab-break | lab-creatures | lab-flood | lab-dominoes) echo lab ;;
    *) echo island ;;
  esac
}
# wanted NAME: true when FORGE_SETS asks for the run NAME.
wanted() {
  [[ $sets == *" all "* ]] && return 0
  local set
  for set in $(sets_of "${1%-fb}"); do
    [[ $sets == *" $set "* ]] && return 0
  done
  return 1
}
summary=$out/summary.txt
exe=""
[ -f "$bin/asteroids.exe" ] && exe=.exe
cd "$root"
log=$(mktemp)
trap 'rm -f "$log"' EXIT
header="tools/validate.sh $bin, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD 2>/dev/null)"
echo "$header"
if [ "$keep" != 0 ]; then
  mkdir -p "$out/logs"
  echo "$header" > "$summary"
fi
# say LINE...: prints, and keeps in the summary with FORGE_KEEP_LOGS=1.
say() {
  echo "$@"
  [ "$keep" != 0 ] && echo "$@" >> "$summary"
  return 0
}

# validate NAME CMD...: the run under the layer, its messages counted; skipped when FORGE_SETS
# leaves it out.
validate() {
  local name=$1
  shift
  wanted "$name" || return 0
  local start=$SECONDS
  say "== $*"
  FORGE_SYNC_VALIDATION=1 "$@" --validate 2>&1 | sed 's/\x1b\[[0-9;]*m//g' > "$log"
  while IFS= read -r line; do say "$line"; done < <(
    grep -E "VUID|SYNC-|Validation (Error|Warning)|ERROR|panicked|mip check|tone check" "$log" |
      grep -v "GOG" | cut -c1-300 | sort | uniq -c | head -n 8)
  say "   $((SECONDS - start)) s, $(wc -l < "$log") log lines"
  [ -s "$log" ] || say "   the run printed nothing: did the demo start?"
  if [ "$keep" != 0 ]; then
    cp "$log" "$out/logs/$name.log"
    grep -E "forge_app: gpu:|selected GPU" "$log" | cut -c1-200 >> "$summary"
  fi
}

for path in $paths; do
  [ "$path" = fb ] && path=--force-fallback || path=""
  tag=${path:+-fb}
  validate "ballad$tag" "$bin/asteroids$exe" --frames 90 $path
  validate "meshlets$tag" "$bin/meshlets$exe" --mip-check --tone-check --frames 60 $path
  validate "city$tag" "$bin/city-blocks$exe" --frames 90 $path
  validate "city-resident$tag" "$bin/city-blocks$exe" --stream-pool 0 --frames 60 $path
  # The flight streams pages on the transfer queue after the start, and the frames after a copy
  # leave out the waits their queue already made (#104).
  validate "city-fly$tag" "$bin/city-blocks$exe" --fly --frames 600 $path
  validate "gallery$tag" "$bin/city-blocks$exe" --gallery --frames 60 $path
  # The island resident on its 8 m ground (the 2 m ground's pages exceed a resident pool, #106),
  # then as it starts by default: at 2 m, streamed.
  validate "island$tag" "$bin/city-blocks$exe" --island 7 --island-drawn 8 --no-water --stream-pool 0 --frames 60 $path
  validate "water$tag" "$bin/city-blocks$exe" --island 7 --island-drawn 8 --stream-pool 0 --frames 60 $path
  validate "island-2m$tag" "$bin/city-blocks$exe" --island 7 --frames 60 $path
  # The cloud layer (#145).
  validate "clouds$tag" "$bin/city-blocks$exe" --island 7 --clouds 0.5 --frames 60 $path
  # Under the sea (#108, the log's `under the sea` view): the water at the camera, the surface
  # from below and the water between.
  validate "under-sea$tag" "$bin/city-blocks$exe" --island 7 --frames 60 --view=4770,-3.0,-2847,91.1,-10 $path
  # And under the largest lake (the log's `under` view of the island's lakes).
  validate "under-lake$tag" "$bin/city-blocks$exe" --island 7 --frames 60 --view=2248,20.2,-1184,0,25 $path
  # Moving geometry (#79): a thousand barrels on the rivers, the first one in view.
  validate "movers$tag" "$bin/city-blocks$exe" --island 7 --frames 60 --movers 1000 --view=-238.2,318.14,-1843.9,135.2,-18.1 $path
  # Splashes (#107): the dropped barrel meets its lake at frame 164, the crown and the jet after.
  validate "splashes$tag" "$bin/city-blocks$exe" --island 7 --frames 200 --fixed-step --movers 1000 --view=2160.0,30.55,-1234.0,0.0,-8.5 $path
  # The island demo (#96): its tour's first 10 s, out of the steep valley, at a time of day
  # whose exposure is metered from the scene.
  [ -f "$bin/island$exe" ] && validate "island-tour$tag" "$bin/island$exe" --tour --fixed-step --time-of-day 0.3 --frames 600 $path
  # The physics lab (#136): its rain landing, a tick a frame.
  [ -f "$bin/physics-lab$exe" ] && validate "lab$tag" "$bin/physics-lab$exe" --fixed-step --frames 120 $path
  [ -f "$bin/physics-lab$exe" ] && validate "lab-sea$tag" "$bin/physics-lab$exe" --lab sea --fixed-step --steer 1,0.6 --frames 120 $path
  [ -f "$bin/physics-lab$exe" ] && validate "lab-walk$tag" "$bin/physics-lab$exe" --lab walk --fixed-step --walk 2,0 --frames 120 $path
  [ -f "$bin/physics-lab$exe" ] && validate "lab-drive$tag" "$bin/physics-lab$exe" --lab drive --fixed-step --steer 1,0.3 --frames 120 $path
  [ -f "$bin/physics-lab$exe" ] && validate "lab-fly$tag" "$bin/physics-lab$exe" --lab fly --fixed-step --pilot 1,-0.4,0,0 --frames 120 $path
  [ -f "$bin/physics-lab$exe" ] && validate "lab-break$tag" "$bin/physics-lab$exe" --lab break --fixed-step --release 1 --frames 120 $path
  [ -f "$bin/physics-lab$exe" ] && validate "lab-creatures$tag" "$bin/physics-lab$exe" --lab creatures --fixed-step --throw-every 50 --frames 120 $path
  [ -f "$bin/physics-lab$exe" ] && validate "lab-flood$tag" "$bin/physics-lab$exe" --lab flood --fixed-step --release 31 --frames 120 $path
  [ -f "$bin/physics-lab$exe" ] && validate "lab-dominoes$tag" "$bin/physics-lab$exe" --lab dominoes --fixed-step --release 31 --frames 120 $path
  # And ships through the belt, the camera chasing the first (#79's demo).
  validate "ships$tag" "$bin/asteroids$exe" --frames 60 --ships 24 --chase 0 $path
  # The HDR output drawn off-screen and previewed (#94), through ACES 2.0: from the TAA resolve
  # (the ballad) and from the stand-alone display pass (the bench). Its MaxCLL and MaxFALL are
  # measured from every frame (#125).
  validate "hdr$tag" "$bin/asteroids$exe" --frames 60 --tonemap aces2 --hdr offscreen $path
  # A calibration page over the frame (#125).
  validate "hdr-calibration$tag" env FORGE_HDR_CALIBRATION=peak "$bin/asteroids$exe" --frames 30 --tonemap aces2 --hdr offscreen $path
  # And switched on and off every 20 frames (F2), the passes rebuilt for each target.
  validate "hdr-switch$tag" env FORGE_HDR_CYCLE=20 "$bin/asteroids$exe" --frames 130 --tonemap aces2 $path
  validate "hdr-display$tag" "$bin/meshlets$exe" --frames 30 --tonemap aces2 --hdr offscreen $path
done
[ "$keep" != 0 ] && say "logs in $out/logs, summary in $summary"
exit 0
