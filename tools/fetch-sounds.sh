#!/usr/bin/env bash
# Fetches the ambience's recordings listed in assets/sounds.tsv into assets/sounds/, which git
# ignores, and checks each file's size and SHA-1:
#
#   tools/fetch-sounds.sh          every recording
#   tools/fetch-sounds.sh birds/serin.ogg   only these
#
# A file already there with the right hash is kept. They are free to ship with their credit
# (CC0 or CC BY, CREDITS.md, "Assets"); forge-audio plays them when they are there and makes
# its birds in code when they are not (docs/demos/audio.md).
set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
manifest=$here/assets/sounds.tsv
out=$here/assets/sounds
agent="forge-engine/1.0 (https://github.com/Eolan/forge-engine)"

wanted=("$@")
picked() {
  [ ${#wanted[@]} -eq 0 ] && return 0
  local w
  for w in "${wanted[@]}"; do [ "$w" = "$1" ] && return 0; done
  return 1
}

failed=0
while IFS=$'\t' read -r file bytes sha1 licence author page url; do
  case $file in '#'* | '') continue ;; esac
  picked "$file" || continue
  path=$out/$file
  if [ -f "$path" ] && [ "$(sha1sum "$path" | cut -d' ' -f1)" = "$sha1" ]; then
    echo "kept    $file"
    continue
  fi
  mkdir -p "$(dirname "$path")"
  curl -sSfL -A "$agent" -o "$path.part" "$url"
  size=$(wc -c < "$path.part" | tr -d ' ')
  sum=$(sha1sum "$path.part" | cut -d' ' -f1)
  if [ "$size" != "$bytes" ] || [ "$sum" != "$sha1" ]; then
    echo "BAD     $file: $size bytes, SHA-1 $sum (expected $bytes, $sha1)" >&2
    rm -f "$path.part"
    failed=1
    continue
  fi
  mv "$path.part" "$path"
  echo "fetched $file ($bytes bytes, $licence, $author)"
done < "$manifest"
exit $failed
