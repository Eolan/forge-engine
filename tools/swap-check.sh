#!/usr/bin/env bash
# How visible each swap of the planet's tiles is (D-056's swap rule, #220): the frames that
# `planet --check-swaps DIR` saved just before and after each swap, the camera held still,
# compared with ꟻLIP. D-017's class 2 passes: the mean below 0.02 and every pixel below 0.15, or
# a larger peak on isolated pixels the error map shows (DIR/swap-NNN-flip.png).
#
#   tools/swap-check.sh DIR
#
# Prints a line a swap (the pixels that differ, ꟻLIP's mean and largest value) and how many
# passed outright; exits 1 when a mean reaches 0.02.
set -euo pipefail
dir=${1:?usage: tools/swap-check.sh DIR}
root=$(cd "$(dirname "$0")/.." && pwd)
imgdiff=$root/target/release/imgdiff
[ -f "$imgdiff.exe" ] && imgdiff=$imgdiff.exe
[ -f "$imgdiff" ] || { echo "missing $imgdiff: build with cargo build --release" >&2; exit 1; }
total=0 under=0 failed=0
for before in "$dir"/swap-*-before.png; do
  [ -e "$before" ] || { echo "no swaps saved in $dir" >&2; exit 1; }
  after=${before%-before.png}-after.png
  [ -f "$after" ] || continue
  name=$(basename "${before%-before.png}")
  out=$("$imgdiff" "$before" "$after" --flip-map "$dir/$name-flip.png" 2>&1 || true)
  count=$(sed -n 's/.*: \([0-9]*\) \/ [0-9]* pixels differ.*/\1/p' <<< "$out")
  mean=$(sed -n 's/^LDR-FLIP.*: mean \([0-9.]*\),.*/\1/p' <<< "$out")
  peak=$(sed -n 's/^LDR-FLIP.* max \([0-9.]*\) at .*/\1/p' <<< "$out")
  total=$((total + 1))
  if [ -z "$mean" ]; then
    echo "$name: ${count:-0} px differ"
    under=$((under + 1))
    continue
  fi
  verdict=
  if awk -v m="$mean" 'BEGIN { exit !(m >= 0.02) }'; then
    verdict=" FAIL (mean)"
    failed=$((failed + 1))
  elif awk -v p="$peak" 'BEGIN { exit !(p >= 0.15) }'; then
    verdict=" (peak: see $name-flip.png)"
  else
    under=$((under + 1))
  fi
  echo "$name: $count px differ, FLIP mean $mean, max $peak$verdict"
done
echo "$total swaps: $under under both thresholds, $failed with a mean of 0.02 or more"
[ "$failed" = 0 ]
