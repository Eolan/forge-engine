#!/usr/bin/env bash
# D-083's calibration: Forge's diffuse light techniques against the references, their error and
# their cost, view by view (D-088).
#
#   tools/gi-compare.sh [OUT_DIR]
#
# For each view:
# - **The captures:** Forge's path tracer (`r.gi.reference=4`) and each technique (`r.gi.cache`
#   off, port, sharc), in linear light at frame 280 of 300, at the view's fixed exposure and
#   without bloom (`--capture-linear`, cd/m²).
# - **The error** of each technique (and of Forge's path against Falcor) against each reference
#   the view has: Forge's path, and Falcor's render where FALCOR_OUT holds it, times its scale
#   to Forge's units. Measured by tools/gi-measure.py: the mean |log2| of the luminance in
#   16 x 9 tiles, its signed mean (negative: darker) and the frame's light over the reference's.
# - **The cost:** each technique FORGE_RUNS times, alternating, so a warm or throttled GPU weighs
#   on all alike. The frame's GPU ms; `gi/trace`; the probes' rays and blend; the port's
#   three passes; SHaRC's update and resolve (the overlay's zones; the resolve's holds the wait
#   for the update, D-088); SHaRC's most slots in use.
# - **A sheet** for the owner: the images tone-mapped alike above their ratio maps against
#   Falcor's render where the view has one, else Forge's path (red brighter, blue darker, full
#   colour at one stop).
#
# OUT_DIR (default captures/gi-compare) gets the captures and logs, sheets/, error.txt,
# timings.txt and summary.md. Settings, as variables:
# - FORGE_VIEWS: views to run, separated by spaces (default all): bistro-e1-clay bistro-e2-clay
#   bistro-e1 bistro-e2 classroom-clay classroom village-landing village-room village-street.
# - FORGE_TECHNIQUES: `r.gi.cache` values (default "off port sharc").
# - FORGE_RUNS: timing runs per technique (default 3; 0 skips the timing).
# - FORGE_TIME_FRAMES: frames per timing run (default 1200: the demos average every frame, and the
#   first, a cache filling from empty, weigh less over more).
# - FORGE_REUSE=1: keep the captures already in OUT_DIR (the path tracer takes minutes a view).
#
# A technique that does not start (`sharc` without `sharc-sdk/` or 64-bit atomics, the port
# without its pool) falls back to the probes with a warning: the tool drops it from the view
# rather than measuring the probes under its name. A failed run is left out of the means.
# - FALCOR_OUT: Falcor's renders, NAME.AccumulatePass.output.0.exr (default ../falcor/out beside
#   the checkout): the Bistro's clay-ext1-lam16, clay-ext2-lam16, ext1-lam16, ext2-lam16 (Lambert
#   diffuse, 16 bounces, × 2,871) and the classroom's crl-clay, crl-tex (× 12,800).
# - BLENDER: Blender's executable (else found on the PATH or in its default Windows place).
#
# Needs the release build (physics-lab, city-plan, contact-sheet) and the scenes' assets in
# assets/external/ (the Bistro and the classroom, fetched by hand, D-083). Every run holds the
# GPU lock (FORGE_GPU_LOCK, default %TEMP%/forge-gpu.lock), as tools/verify.sh does. The owner's
# settings file (forge.cfg, D-069) never changes a scripted run. About an hour for every view.
export FORGE_CFG=none FORGE_SOUND=0
export FORGE_MONITOR=${FORGE_MONITOR:-secondary}
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
out=${1:-$root/captures/gi-compare}
views=${FORGE_VIEWS:-bistro-e1-clay bistro-e2-clay bistro-e1 bistro-e2 classroom-clay classroom village-landing village-room village-street}
techniques=${FORGE_TECHNIQUES:-off port sharc}
runs=${FORGE_RUNS:-3}
time_frames=${FORGE_TIME_FRAMES:-1200}
reuse=${FORGE_REUSE:-0}
falcor=${FALCOR_OUT:-$(dirname "$root")/falcor/out}
bin=$root/target/release
exe=""
[ -f "$bin/physics-lab.exe" ] && exe=.exe
[ -f "$bin/physics-lab$exe" ] && [ -f "$bin/city-plan$exe" ] && [ -f "$bin/contact-sheet$exe" ] ||
  { echo "build first: cargo build --release" >&2; exit 2; }
blender=${BLENDER:-}
if [ -z "$blender" ]; then
  if command -v blender > /dev/null; then
    blender=blender
  else
    blender=$(ls -d "/c/Program Files/Blender Foundation"/Blender*/blender.exe 2> /dev/null | tail -1)
  fi
fi
[ -n "$blender" ] || { echo "Blender not found: set BLENDER" >&2; exit 2; }
if [ -n "${FORGE_GPU_LOCK:-}" ]; then
  lock=$FORGE_GPU_LOCK
elif [ -n "${LOCALAPPDATA:-}" ] && command -v cygpath > /dev/null; then
  lock=$(cygpath -u "$LOCALAPPDATA")/Temp/forge-gpu.lock
else
  lock=${TMPDIR:-/tmp}/forge-gpu.lock
fi
locked=0
unlock() {
  [ $locked = 1 ] && rm -rf "$lock"
  locked=0
}
trap unlock EXIT
trap 'exit 130' INT TERM
lock_gpu() {
  [ $locked = 1 ] && return 0
  until mkdir "$lock" 2> /dev/null; do
    echo "the GPU is busy ($(cat "$lock/owner" 2> /dev/null || echo "no owner file")): retrying in 30 s"
    sleep 30
  done
  echo "tools/gi-compare.sh in $root, pid $$, since $(date -u +%FT%TZ)" > "$lock/owner"
  locked=1
}
# label NAME: a capture's name on the sheets.
label_of() {
  case $1 in
    path) echo "Forge path" ;;
    off) echo "probes (off)" ;;
    sharc) echo "SHaRC" ;;
    *) echo "$1" ;;
  esac
}
# win PATH: the path as the Windows binaries and Blender take it.
win() { if command -v cygpath > /dev/null; then cygpath -m "$1"; else echo "$1"; fi; }
# measure ARGS...: tools/gi-measure.py in Blender. Its lines when it succeeds; else Blender's last
# error lines on stderr, and a failure.
measure() {
  local said code
  said=$("$blender" -b --factory-startup --python-exit-code 1 --python "$(win "$root/tools/gi-measure.py")" -- "$@" 2>&1)
  code=$?
  if [ $code != 0 ]; then
    echo "   gi-measure.py $1 failed (exit $code):" >&2
    echo "$said" | grep -E 'Error|error|Traceback' | tail -n 5 | sed 's/^/     /' >&2
    return 1
  fi
  echo "$said"
}
# fell_back LOG: true when the run's technique did not start and the probes stood in.
fell_back() { [ -f "$1" ] && grep -qE "no SHaRC|no surface cache" "$1"; }

E1="--view=-23.700,6.855,-0.704,-93.13,-15.93 --fov 36.22"
E2="--view=13.378,2.280,11.709,48.33,-0.50 --fov 38.90"
CLASSROOM="--view=-1.6905,1.2716,5.8865,-8.705,1.622 --fov 35.9834"
VILLAGE="--village 600 --time-of-day 0.75 --no-fauna"
# describe VIEW: sets the view's demo, its flags, clay (1: every surface a white diffuse), its
# fixed exposure and its Falcor reference (a file stem in FALCOR_OUT and its scale), if any.
describe() {
  clay="" falcor_ref="" falcor_scale=1
  case $1 in
    bistro-e1-clay) demo=physics-lab flags="--lab bistro $E1" clay=1 ev=11.22 falcor_ref=clay-ext1-lam16 falcor_scale=2871 ;;
    bistro-e2-clay) demo=physics-lab flags="--lab bistro $E2" clay=1 ev=11.22 falcor_ref=clay-ext2-lam16 falcor_scale=2871 ;;
    bistro-e1) demo=physics-lab flags="--lab bistro $E1" ev=11.22 falcor_ref=ext1-lam16 falcor_scale=2871 ;;
    bistro-e2) demo=physics-lab flags="--lab bistro $E2" ev=11.22 falcor_ref=ext2-lam16 falcor_scale=2871 ;;
    classroom-clay) demo=physics-lab flags="--lab classroom $CLASSROOM" clay=1 ev=9 falcor_ref=crl-clay falcor_scale=12800 ;;
    classroom) demo=physics-lab flags="--lab classroom $CLASSROOM" ev=9 falcor_ref=crl-tex falcor_scale=12800 ;;
    village-landing) demo=city-plan flags="$VILLAGE --view=75.2,5.2,-175.7,146.8,-25" ev=9 ;;
    village-room) demo=city-plan flags="$VILLAGE --view=71.5,1.6,-175.1,55.9,-12" ev=9 ;;
    village-street) demo=city-plan flags="$VILLAGE --view=-79.8,1.7,-272.0,-98.3,0" ev=9 ;;
    *) echo "unknown view $1" >&2; exit 2 ;;
  esac
}
for v in $views; do describe "$v"; done
# run VIEW LOG FRAMES [FLAGS...]: the view's demo for FRAMES frames under the GPU lock.
run() {
  local view=$1 log=$2 frames=$3
  shift 3
  describe "$view"
  lock_gpu
  # shellcheck disable=SC2086 # the flags are words
  env ${clay:+FORGE_BISTRO_CLAY=1} timeout 1800 "$bin/$demo$exe" $flags --fixed-step --frames "$frames" \
    --ev100 "$ev" --fixed-exposure --set r.bloom=0 "$@" > "$log" 2>&1
}
# capture VIEW NAME FLAGS...: the linear capture at frame 280 of 300, unless kept. Fails when the
# run fails, saves nothing or its technique fell back to the probes.
capture() {
  local view=$1 name=$2
  shift 2
  local base=$out/$view/$name
  if [ "$reuse" = 1 ] && [ -f "$base.hdr" ] && ! fell_back "$base.log"; then
    echo "   kept $view/$name"
    return
  fi
  rm -f "$base.hdr" "$base.png"
  run "$view" "$base.log" 300 --capture-frame 280 --capture "$(win "$base.png")" \
    --capture-linear "$(win "$base.hdr")" "$@"
  local code=$?
  if fell_back "$base.log"; then
    echo "   $view/$name: the technique did not start (the probes stood in): left out"
    return 1
  fi
  echo "   $view/$name: exit $code"
  [ $code = 0 ] && [ -f "$base.hdr" ]
}
# zones LOG: the run's GPU ms, `frame trace probes port sharc_update sharc_resolve most` (most:
# SHaRC's slots in use at most, 0 without it). Fails when the run printed no times.
zones() {
  local clean line most
  clean=$(sed 's/\x1b\[[0-9;]*m//g' "$1")
  line=$(echo "$clean" | grep -oE 'gpu: [0-9.]+ ms per frame over [0-9]+ frames: .*' | head -1)
  [ -n "$line" ] || return 1
  most=$(echo "$clean" | grep "SHaRC's slots" | grep -oE 'most=[0-9]+' | head -1 | cut -d= -f2)
  echo "$line" | awk -v most="${most:-0}" '{
    s = $0; sub(/^gpu: /, "", s); split(s, head, " ms per frame"); frame = head[1]
    sub(/^[^:]*: /, "", s); n = split(s, z, ", ")
    trace = probes = port = upd = res = 0
    for (i = 1; i <= n; i++) {
      v = z[i]; ms = v; sub(/.* /, "", ms); name = v; sub(/ [0-9.]+$/, "", name)
      sub(/ \[compute\]$/, "", name)
      if (name == "gi/trace") trace = ms
      else if (name == "gi/probe rays" || name == "gi/probe blend") probes += ms
      else if (name ~ /^gi\/cache (age|rays|blend)$/) port += ms
      else if (name == "gi/sharc update") upd = ms
      else if (name == "gi/sharc resolve") res = ms
    }
    printf "%s %.3f %.3f %.3f %.3f %.3f %s\n", frame, trace, probes, port, upd, res, most
  }'
}

mkdir -p "$out/sheets"
error=$out/error.txt timings=$out/timings.txt
: > "$error"
: > "$timings"
start=$SECONDS
for view in $views; do
  describe "$view"
  mkdir -p "$out/$view"
  echo "== $view ($((SECONDS - start)) s)"
  capture "$view" path --set r.gi.reference=4 || { echo "   no path capture: $view skipped"; continue; }
  tests=()
  for t in $techniques; do
    capture "$view" "$t" --set r.gi.cache="$t" && tests+=("$t")
  done
  # The references: Forge's path, then Falcor's render when there is one.
  refs=("Forge path=$out/$view/path.hdr*1")
  falcor_exr=$falcor/$falcor_ref.AccumulatePass.output.0.exr
  if [ -n "$falcor_ref" ]; then
    if [ -f "$falcor_exr" ]; then
      refs+=("Falcor=$falcor_exr*$falcor_scale")
    else
      echo "   no Falcor render $falcor_exr: Forge's path alone"
    fi
  fi
  for ref in "${refs[@]}"; do
    label=${ref%%=*} path=${ref#*=}
    items=()
    [ "$label" != "Forge path" ] && items+=("$out/$view/path.hdr")
    for t in "${tests[@]}"; do items+=("$out/$view/$t.hdr"); done
    [ ${#items[@]} -gt 0 ] || continue
    winitems=()
    for i in "${items[@]}"; do winitems+=("$(win "$i")"); done
    if said=$(measure tiles "$(win "${path%'*'*}")" "${path##*'*'}" "${winitems[@]}"); then
      echo "$said" | grep '^TIL' | sed "s|^TIL |$view vs $label: |" | tee -a "$error"
    else
      echo "   no error measured against $label"
    fi
  done
  # The sheet: the first reference, the tests, each tone-mapped alike, their ratio maps under.
  d=$out/sheets/$view
  mkdir -p "$d"
  rm -f "$d"/*.png "$out/sheets/$view.png"
  ref=${refs[-1]}
  label=${ref%%=*} path=${ref#*=}
  tm=("ref=$(win "${path%'*'*}")*${path##*'*'}") rm=() labels=(--label "$label") imgs=("$(win "$d/ref.png")") maps=("$(win "$d/blank.png")")
  names=()
  [ "$label" != "Forge path" ] && names+=(path)
  names+=("${tests[@]}")
  for n in "${names[@]}"; do
    tm+=("$n=$(win "$out/$view/$n.hdr")")
    rm+=("$n=$(win "$out/$view/$n.hdr")")
    labels+=(--label "$(label_of "$n")")
    imgs+=("$(win "$d/$n.png")")
    maps+=("$(win "$d/map-$n.png")")
  done
  rmap=()
  for r in "${rm[@]}"; do rmap+=("map-$r"); done
  mlabels=(--label "red brighter, blue darker (1 stop)")
  for n in "${names[@]}"; do mlabels+=(--label "vs $label"); done
  if measure tonemap "$(win "$d")" 0.18 "${tm[@]}" > /dev/null &&
    measure ratio "$(win "$d")" "$(win "${path%'*'*}")" "${path##*'*'}" 8 "${rmap[@]}" > /dev/null &&
    measure blank "$(win "$d/blank.png")" "$(win "$d/ref.png")" > /dev/null &&
    "$bin/contact-sheet$exe" "$(win "$out/sheets/$view.png")" "${imgs[@]}" "${maps[@]}" \
      --columns $((${#names[@]} + 1)) --width 480 "${labels[@]}" "${mlabels[@]}" > /dev/null; then
    echo "   sheet sheets/$view.png"
  else
    echo "   no sheet for $view"
  fi
  # The cost: each technique that started, in turn, FORGE_RUNS times.
  for ((i = 1; i <= runs; i++)); do
    for t in "${tests[@]}"; do
      log=$out/$view/time-$t-$i.log
      if run "$view" "$log" "$time_frames" --set r.gi.cache="$t" && ! fell_back "$log" &&
        row=$(zones "$log"); then
        echo "$view $t $row" >> "$timings"
      else
        echo "   $view/$t timing run $i failed or fell back: left out ($log)"
      fi
    done
  done
done
unlock

# The summary: the error table, then each view's and technique's mean cost.
{
  echo "# tools/gi-compare.sh, $(date -u +%FT%TZ), commit $(git rev-parse --short HEAD 2> /dev/null)"
  echo
  echo "Mean |log2| of 16 x 9 tiles (signed mean; the frame's light over the reference's):"
  echo
  echo "| View | reference | image | error | signed | frame |"
  echo "|---|---|---|---|---|---|"
  sed -E 's/^([^ ]+) vs ([^:]+): +([^ ]+)\.hdr +\|log2\| ([0-9.]+) +signed ([-+0-9.]+) +frame (x[0-9.]+)/| \1 | \2 | \3 | \4 | \5 | \6 |/' "$error"
  if [ -s "$timings" ]; then
    echo
    echo "GPU ms over $time_frames frames, the mean of the runs that succeeded (SHaRC's resolve zone"
    echo "holds the wait for its update):"
    echo
    echo "| View | r.gi.cache | runs | frame | gi/trace | probes | port | SHaRC update | SHaRC resolve | SHaRC's slots at most |"
    echo "|---|---|---|---|---|---|---|---|---|---|"
    awk 'NF == 9 { k = $1 " " $2; if (!(k in n)) order[++m] = k; n[k]++
      for (i = 3; i <= 8; i++) s[k, i] += $i; most[k] = $9 }
      END { for (j = 1; j <= m; j++) { k = order[j]; split(k, p, " ")
        printf "| %s | %s | %d |", p[1], p[2], n[k]
        for (i = 3; i <= 8; i++) printf " %.3f |", s[k, i] / n[k]
        printf " %s |\n", most[k] } }' "$timings"
  fi
} > "$out/summary.md"
echo "== done ($((SECONDS - start)) s): $out/summary.md"
