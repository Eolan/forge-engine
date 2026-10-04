#!/usr/bin/env bash
# Fetches the labs' external models (D-048) listed in assets/external.tsv into
# assets/external/, which git ignores, and checks each file's size and SHA-256:
#
#   tools/fetch-assets.sh                    the open models (CC0 or CC-BY 4.0)
#   tools/fetch-assets.sh --reference-only   the reference models too (restricted licences)
#   tools/fetch-assets.sh Fox Sponza         only these (a reference model still needs the flag)
#
# A file already there with the right hash is kept. The models are for the labs and the tests
# only: nothing that ships reads assets/external/. The reference models' licences restrict
# their use: read assets/external/<model>/LICENSE.md before using one, and never put one in the
# engine or a game.
set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
manifest=$here/assets/external.tsv
out=$here/assets/external
reference=0
wanted=()
for arg in "$@"; do
  case $arg in
    --reference-only) reference=1 ;;
    -h | --help)
      sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    -*) echo "unknown option $arg" >&2; exit 2 ;;
    *) wanted+=("$arg") ;;
  esac
done

commit=$(sed -n 's/^commit //p' "$manifest")
source=$(sed -n 's/^source //p' "$manifest")
declare -A use=()
while IFS=$'\t' read -r tag model kind _; do
  [ "$tag" = "@" ] && use[$model]=$kind
done < "$manifest"

# picked MODEL: true when MODEL is to be fetched.
picked() {
  local model=$1
  if [ ${#wanted[@]} -gt 0 ]; then
    local w found=0
    for w in "${wanted[@]}"; do [ "$w" = "$model" ] && found=1; done
    [ $found = 1 ] || return 1
  fi
  if [ "${use[$model]}" = reference ] && [ $reference = 0 ]; then
    return 1
  fi
  return 0
}

for w in "${wanted[@]}"; do
  [ -n "${use[$w]:-}" ] || { echo "no model $w in $manifest" >&2; exit 2; }
  if [ "${use[$w]}" = reference ] && [ $reference = 0 ]; then
    echo "$w is a reference model under a restricted licence: add --reference-only" >&2
    exit 2
  fi
done

fetched=0 kept=0 bytes=0 failed=0
declare -A shown=()
while IFS=$'\t' read -r model path size sha; do
  case $model in '' | '#'* | '@' | commit* | source*) continue ;; esac
  picked "$model" || continue
  if [ -z "${shown[$model]:-}" ]; then
    echo "== $model (${use[$model]})"
    shown[$model]=1
  fi
  file=$out/$path
  if [ -f "$file" ] && [ "$(sha256sum "$file" | cut -d' ' -f1)" = "$sha" ]; then
    kept=$((kept + 1))
    continue
  fi
  mkdir -p "$(dirname "$file")"
  if ! curl -sfL --retry 3 -o "$file.part" "$source/$commit/Models/$path"; then
    echo "   failed to download $path" >&2
    rm -f "$file.part"
    failed=$((failed + 1))
    continue
  fi
  got=$(sha256sum "$file.part" | cut -d' ' -f1)
  if [ "$got" != "$sha" ] || [ "$(stat -c %s "$file.part")" != "$size" ]; then
    echo "   $path does not match the manifest (sha256 $got)" >&2
    rm -f "$file.part"
    failed=$((failed + 1))
    continue
  fi
  mv "$file.part" "$file"
  fetched=$((fetched + 1))
  bytes=$((bytes + size))
done < "$manifest"

echo "fetched $fetched files ($((bytes / 1048576)) MiB), kept $kept, failed $failed, into $out"
[ $reference = 1 ] || echo "(reference models left out: --reference-only fetches them)"
[ $failed = 0 ]
