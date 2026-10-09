#!/usr/bin/env bash
# Compares two capture batches written by tools/captures.sh (issue #74; docs/PROCESS.md).
#
#   tools/compare.sh [BASE] NEW
#
# Without BASE, NEW is compared with the latest accepted set (issue #134): the newest commit of
# HEAD's history with a set in captures/accepted/<sha>/ (FORGE_ACCEPTED, see tools/verify.sh).
#
# Prints, per image both batches hold, the pixels that differ by more than 2 levels (imgdiff's
# default tolerance) and the largest channel error; for a difference, also its FLIP mean and
# largest value (issue #75: how visible it is; HDR-FLIP for the HDR10 captures, #126). Then the
# pairs within NEW that must match: the A/B harness (occlusion and cone culling off against on,
# `--show-culled` against the plain frame: no red), the island streamed from its start view
# against resident (#121) and the mesh path against the fallback. Exit code 1 when any image of
# either list differs, so a script can stop on it. "0 px" on every line is the pass. With
# FORGE_KEEP_LOGS=1 the same lines go to NEW/compare.txt, for a cloud session to read
# (tools/report.sh gathers it).
#
# Known flake (#71): the ballad's TAA frame 600 on either path (fb-ast-taa600, mesh-ast-taa600)
# can differ by a few hundred scattered edge pixels from the same build, FLIP mean <= 0.0015;
# some days on nearly every run (docs/PROCESS.md, "Known flake"). A difference of those images
# with its signature (at most 500 px, FLIP mean at most 0.0015 and largest at most 0.15: no line,
# speck or patch, which reach 0.17 and more) prints "FLAKE #71" and does not fail. The HDR
# output's frame 600 has TAA on too and flakes the same way (#134): its SDR preview
# (*-ast-hdr600) by the same signature, and its PQ codes (*-ast-hdr600-pq, ~100 000 codes apart
# in the dark) only when the preview flaked too, with HDR-FLIP mean at most 0.005 and largest
# below 0.22 (three flakes: 0.0034-0.0044 and 0.16-0.19; on 2026-10-03 eight more, largest
# 0.14-0.201, and a line of 20 codes reaches 0.24). Both
# HDR captures' PQ codes (*-ast-hdr240-pq, *-ast-hdr600-pq) can also differ by one code on a few
# dozen pixels with the preview the same (2026-10-03, two Tier 2 runs: 20 and 19 px, HDR-FLIP
# largest 0.013-0.016): at most 100 px of one code, largest below 0.02, is the flake too. The
# physics lab's dominoes at frame 3000 (*-lab-dominoes3000, #146) flake the same way, judged by
# the same signature: on 2026-10-03 two runs of one build gave 1 to 3 px apart (max 4 levels,
# FLIP mean 0.00001) with the physics' digest the same to the bit.
# Since TAA's image is sharpened and its history goes through Lanczos-3 (D-045), the ballad's
# frame 600 flakes larger: three runs of one build on 2026-10-03 were 1063-1121 px apart, FLIP
# mean 0.0023-0.0024, largest 0.08-0.09, and the PQ codes' HDR-FLIP mean 0.0063. So for the
# ballad's frame 600 the signature is at most 1500 px and a mean at most 0.003 (PQ: 0.008).
# The PQ codes' largest error then reached 0.284 (a Tier 2 run, 2026-10-03: a few scattered
# pixels on one rock's edge, the preview's own 0.078), so its bound is 0.30; the preview, whose
# signature must hold too, is what tells a shape.
#
# FORGE_EXPECT: the images a change is meant to alter, as patterns separated by spaces or commas
# (for example 'mesh-island* mesh-shot-*'): their differences print "expected" and do not fail.
# The pairs within NEW must match whatever it says.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
usage="usage: tools/compare.sh [BASE] NEW"
if [ $# -ge 2 ]; then
  base=$1
  new=$2
else
  new=${1:?$usage}
  accepted=${FORGE_ACCEPTED:-$root/captures/accepted}
  base=""
  for sha in $(git -C "$root" rev-list -n 500 HEAD); do
    [ -f "$accepted/$sha/manifest.txt" ] && base=$accepted/$sha && break
  done
  [ -n "$base" ] || { echo "no accepted set in $accepted for HEAD's history: give BASE" >&2; exit 1; }
fi
expect=${FORGE_EXPECT:-}
expect=${expect//,/ }
imgdiff=$root/target/release/imgdiff
[ -f "$imgdiff.exe" ] && imgdiff=$imgdiff.exe
[ -f "$imgdiff" ] || { echo "missing $imgdiff: build with cargo build --release" >&2; exit 1; }

status=0
flakes=0
expected=0
missing=""
# flake NAME COUNT MEAN PEAK: true when a difference of NAME against its base has #71's
# signature.
flake() {
  case $1 in
    *-ast-taa600 | *-ast-hdr600)
      awk -v n="$2" -v mean="$3" -v peak="$4" \
        'BEGIN { exit !(n != "" && mean != "" && peak != "" && n <= 1500 && mean <= 0.003 && peak <= 0.15) }'
      ;;
    *-lab-dominoes3000)
      awk -v n="$2" -v mean="$3" -v peak="$4" \
        'BEGIN { exit !(n != "" && mean != "" && peak != "" && n <= 500 && mean <= 0.0015 && peak <= 0.15) }'
      ;;
    *-ast-hdr240-pq | *-ast-hdr600-pq)
      # One code on a few pixels, which the preview's 8 bits do not show (`$5`, the largest
      # error as `pair` prints it).
      if [ "${5:-}" = "1 codes" ] && awk -v n="$2" -v peak="$4" \
        'BEGIN { exit !(n != "" && peak != "" && n <= 100 && peak < 0.02) }'; then
        return 0
      fi
      [[ $1 == *-ast-hdr600-pq ]] || return 1
      awk -v mean="$3" -v peak="$4" \
        'BEGIN { exit !(mean != "" && peak != "" && mean <= 0.008 && peak < 0.30) }' || return 1
      preview_flaked "${1%-pq}"
      ;;
    *) return 1 ;;
  esac
}
# preview_flaked NAME: true when the SDR image NAME differs from its base with #71's signature.
preview_flaked() {
  local out count mean peak
  [ -f "$base/$1.png" ] && [ -f "$new/$1.png" ] || return 1
  out=$("$imgdiff" "$base/$1.png" "$new/$1.png" 2>&1)
  count=$(sed -n 's/.*: \([0-9]*\) \/ [0-9]* pixels differ.*/\1/p' <<< "$out")
  mean=$(sed -n 's/^LDR-FLIP.*: mean \([0-9.]*\),.*/\1/p' <<< "$out")
  peak=$(sed -n 's/^LDR-FLIP.* max \([0-9.]*\) at .*/\1/p' <<< "$out")
  [ "$count" != 0 ] && flake "$1" "$count" "$mean" "$peak"
}
# is_expected NAME: true when NAME matches a pattern of FORGE_EXPECT.
is_expected() {
  local pattern patterns
  # Split on spaces only: unquoted, `*` took the names of the files where the script runs.
  read -ra patterns <<< "$expect"
  for pattern in "${patterns[@]}"; do
    # shellcheck disable=SC2053 # the pattern is a glob on purpose
    [[ $1 == $pattern ]] && return 0
  done
  return 1
}
# pair A B LABEL [same]: one line with the count of differing pixels and the largest error, and
# for a difference its perceptual error, FLIP's mean and largest value (issue #75). The HDR10
# captures' PQ codes (16 bits, #94) must match to the code; their largest error is in 10-bit
# codes, and their FLIP is HDR-FLIP on their light (#126). With "same" (an image against its
# base), the flake (#71) and FORGE_EXPECT's images are told apart and do not fail.
pair() {
  if [ "${4:-}" = same ] && [ ! -f "$1" ] && [ -f "$2" ]; then
    missing="$missing $3"
    return 0
  fi
  [ -f "$1" ] && [ -f "$2" ] || return 0
  local out line flip count max mean peak label=FLIP tolerance=()
  [[ "$2" == *-pq.png ]] && tolerance=(--tolerance 0)
  out=$("$imgdiff" "$1" "$2" "${tolerance[@]}" 2>&1)
  line=$(grep "pixels differ" <<< "$out" | tail -n 1)
  flip=$(grep -E "^(LDR|HDR)-FLIP" <<< "$out" | tail -n 1)
  [[ "$flip" == HDR-FLIP* ]] && label=HDR-FLIP
  count=$(sed -n 's/.*: \([0-9]*\) \/ [0-9]* pixels differ.*/\1/p' <<< "$line")
  max=$(sed -n 's/.*max channel error \([0-9]*\).*/\1/p' <<< "$line")
  [[ "$line" == *"PQ code error"* ]] && max="$(sed -n 's/.*max PQ code error \([0-9]*\).*/\1/p' <<< "$line") codes"
  mean=$(sed -n 's/.*: mean \([0-9.]*\),.*/\1/p' <<< "$flip")
  peak=$(sed -n 's/.* max \([0-9.]*\) at .*/\1/p' <<< "$flip")
  if [ -z "$count" ]; then
    echo "$3: imgdiff failed"
    status=1
  elif [ "$count" != 0 ]; then
    line="$3: $count px differ (max $max), $label mean $mean, max $peak"
    if [ "${4:-}" = same ] && flake "$3" "$count" "$mean" "$peak" "$max"; then
      echo "$line: FLAKE #71"
      flakes=$((flakes + 1))
    elif [ "${4:-}" = same ] && is_expected "$3"; then
      echo "$line: expected"
      expected=$((expected + 1))
    else
      echo "$line"
      status=1
    fi
  else
    echo "$3: 0 px"
  fi
}

main() {
  echo "tools/compare.sh $base $new, $(date -u +%FT%TZ)"
  echo "== $base against $new"
  local images=("$new"/*.png)
  [ -f "${images[0]}" ] || { echo "no images in $new"; status=1; }
  for image in "${images[@]}"; do
    [ -f "$image" ] || continue
    name=$(basename "$image" .png)
    pair "$base/$name.png" "$image" "$name" same
  done
  echo "== within $new: the A/B harness and mesh against fallback"
  for path in mesh fb; do
    pair "$new/$path-orbit120.png" "$new/$path-noocc120.png" "$path orbit, occlusion off"
    pair "$new/$path-ast240.png" "$new/$path-ast240-noocc.png" "$path ballad, occlusion off"
    pair "$new/$path-ast240.png" "$new/$path-ast240-nocone.png" "$path ballad, cone off"
    pair "$new/$path-ast240.png" "$new/$path-ast240-culled.png" "$path ballad, show-culled"
    pair "$new/$path-city60.png" "$new/$path-city60-noocc.png" "$path city, occlusion off"
    pair "$new/$path-city60.png" "$new/$path-city60-culled.png" "$path city, show-culled"
    pair "$new/$path-island60.png" "$new/$path-island60-noocc.png" "$path island, occlusion off"
    pair "$new/$path-water60.png" "$new/$path-water60-noocc.png" "$path island with water, occlusion off"
    pair "$new/$path-clouds60.png" "$new/$path-clouds60-noocc.png" "$path island with clouds, occlusion off"
    pair "$new/$path-island8-60.png" "$new/$path-island8-60-resident.png" "$path island at 8 m, streamed against resident"
    pair "$new/$path-lab-drop90.png" "$new/$path-lab-drop90-noocc.png" "$path lab, occlusion off"
    pair "$new/$path-lab-sea300.png" "$new/$path-lab-sea300-noocc.png" "$path lab's sea, occlusion off"
    pair "$new/$path-lab-walk150.png" "$new/$path-lab-walk150-noocc.png" "$path lab's playground, occlusion off"
    pair "$new/$path-lab-drive300.png" "$new/$path-lab-drive300-noocc.png" "$path lab's track, occlusion off"
    pair "$new/$path-lab-fly1200.png" "$new/$path-lab-fly1200-noocc.png" "$path lab's field, occlusion off"
    pair "$new/$path-lab-break85.png" "$new/$path-lab-break85-noocc.png" "$path lab's wall, occlusion off"
    pair "$new/$path-lab-creatures120.png" "$new/$path-lab-creatures120-noocc.png" "$path lab's creatures, occlusion off"
    pair "$new/$path-lab-flood150.png" "$new/$path-lab-flood150-noocc.png" "$path lab's flood, occlusion off"
    pair "$new/$path-lab-dominoes900.png" "$new/$path-lab-dominoes900-noocc.png" "$path lab's dominoes, occlusion off"
    pair "$new/$path-lab-bridge360.png" "$new/$path-lab-bridge360-noocc.png" "$path lab's bridge, occlusion off"
    pair "$new/$path-lab-rocket120.png" "$new/$path-lab-rocket120-noocc.png" "$path lab's rocket, occlusion off"
    pair "$new/$path-lab-tug-net200.png" "$new/$path-lab-tug-net200-noocc.png" "$path lab's tug-of-war, occlusion off"
    pair "$new/$path-lab-space150.png" "$new/$path-lab-space150-noocc.png" "$path lab's spaceship, occlusion off"
    pair "$new/$path-lab-tank90.png" "$new/$path-lab-tank90-noocc.png" "$path lab's glass tank, occlusion off"
    pair "$new/$path-planet-ground.png" "$new/$path-planet-ground-noocc.png" "$path planet over Èze, occlusion off"
    pair "$new/$path-planet-ground.png" "$new/$path-planet-ground-resident.png" "$path planet over Èze, streamed against resident"
  done
  for name in static60 orbit120 nolod120 ast240 ast-notaa600 ast-hdr240 ast-hdr240-pq city60 cityorbit120 gallery60 island60 water60 clouds60 \
    shot-mouth shot-lake shot-island shot-valley lab-drop90 lab-drop600 lab-net300 lab-sea300 lab-sea-steer600 lab-walk150 lab-walk-crates240 \
    lab-drive300 lab-drive-turn600 lab-fly1200 lab-break85 lab-break300 lab-creatures120 \
    lab-creatures-throw240 lab-creatures-limp240 lab-creatures-dq60 lab-creatures-corrective60 lab-creatures-slime8-60 lab-course600 lab-course1200 lab-flyer300 lab-flyer600 lab-yard240 lab-yard600 lab-yard1200 lab-flood150 lab-flood300 \
    lab-flood150-columns lab-dominoes900 \
    lab-bridge360 lab-bridge600 lab-rocket120 lab-rocket600 lab-tug-net200 lab-tug-net600 lab-space90 lab-space150 \
    lab-tank90 lab-tank-bench300 lab-tank-hole120 lab-tank-blocks56 lab-room60 lab-room-pan60 lab-room-pan60-ssaa \
    lab-models60 lab-model-CesiumMan lab-model-FlightHelmet lab-model-Fox lab-model-MetalRoughSpheres \
    lab-model-NormalTangentMirrorTest lab-model-NormalTangentTest lab-model-TextureCoordinateTest \
    lab-model-TextureSettingsTest lab-model-TextureTransformTest lab-model-WaterBottle lab-model-Sponza \
    lab-model-Sponza-noprobes planet-orbit planet-high planet-ground planet-moon-orbit planet-moon-ground; do
    pair "$new/mesh-$name.png" "$new/fb-$name.png" "mesh against fallback, $name"
  done
  local others=""
  [ $flakes != 0 ] && others="$others, $flakes the flake (#71)"
  [ $expected != 0 ] && others="$others, $expected expected (FORGE_EXPECT)"
  [ -n "$missing" ] && echo "not in $base, not compared ($(wc -w <<< "$missing")):$missing"
  if [ $status = 0 ] && [ -z "$others" ]; then
    echo "every line is 0 px: the pass"
  elif [ $status = 0 ]; then
    echo "every other line is 0 px$others: the pass"
  else
    echo "some lines differ: named above"
  fi
  return $status
}

if [ "${FORGE_KEEP_LOGS:-0}" != 0 ]; then
  main | tee "$new/compare.txt"
  exit "${PIPESTATUS[0]}"
fi
main
