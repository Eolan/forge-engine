#!/usr/bin/env bash
# Compares two capture batches written by tools/captures.sh (issue #74; docs/PROCESS.md).
#
#   tools/compare.sh BASE NEW
#
# Prints, per image both batches hold, the pixels that differ by more than 2 levels (imgdiff's
# default tolerance) and the largest channel error; for a difference, also its LDR-FLIP mean and
# largest value (issue #75: how visible it is). Then the pairs within NEW that must match:
# the A/B harness (occlusion and cone culling off against on, `--show-culled` against the plain
# frame: no red) and the mesh path against the fallback. Exit code 1 when any image of either
# list differs, so a script can stop on it.
#
# Known flake (#71): the fallback's TAA frame 600 (fb-ast-taa600) can differ by a few hundred
# pixels from the same build; rerun its capture before looking further.
set -uo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
base=${1:?usage: tools/compare.sh BASE NEW}
new=${2:?usage: tools/compare.sh BASE NEW}
imgdiff=$root/target/release/imgdiff
[ -f "$imgdiff.exe" ] && imgdiff=$imgdiff.exe
[ -f "$imgdiff" ] || { echo "missing $imgdiff: build with cargo build --release" >&2; exit 1; }

status=0
# pair A B LABEL: one line with the count of differing pixels and the largest error, and for a
# difference its perceptual error, LDR-FLIP's mean and largest value (issue #75).
pair() {
  [ -f "$1" ] && [ -f "$2" ] || return 0
  local out line flip count max mean peak
  out=$("$imgdiff" "$1" "$2" 2>&1)
  line=$(grep "pixels differ" <<< "$out" | tail -n 1)
  flip=$(grep "LDR-FLIP" <<< "$out" | tail -n 1)
  count=$(sed -n 's/.*: \([0-9]*\) \/ [0-9]* pixels differ.*/\1/p' <<< "$line")
  max=$(sed -n 's/.*max channel error \([0-9]*\).*/\1/p' <<< "$line")
  mean=$(sed -n 's/.*: mean \([0-9.]*\),.*/\1/p' <<< "$flip")
  peak=$(sed -n 's/.* max \([0-9.]*\) at .*/\1/p' <<< "$flip")
  if [ -z "$count" ]; then
    echo "$3: imgdiff failed"
    status=1
  elif [ "$count" != 0 ]; then
    echo "$3: $count px differ (max $max), FLIP mean $mean, max $peak"
    status=1
  else
    echo "$3: 0 px"
  fi
}

echo "== $base against $new"
for image in "$new"/*.png; do
  name=$(basename "$image" .png)
  pair "$base/$name.png" "$image" "$name"
done
echo "== within $new: the A/B harness and mesh against fallback"
for path in mesh fb; do
  pair "$new/$path-orbit120.png" "$new/$path-noocc120.png" "$path orbit, occlusion off"
  pair "$new/$path-ast240.png" "$new/$path-ast240-noocc.png" "$path ballad, occlusion off"
  pair "$new/$path-ast240.png" "$new/$path-ast240-nocone.png" "$path ballad, cone off"
  pair "$new/$path-ast240.png" "$new/$path-ast240-culled.png" "$path ballad, show-culled"
done
for name in static60 orbit120 nolod120 ast240 ast-notaa600 city60 cityorbit120 gallery60; do
  pair "$new/mesh-$name.png" "$new/fb-$name.png" "mesh against fallback, $name"
done
exit $status
