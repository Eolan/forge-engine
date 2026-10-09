#!/usr/bin/env bash
# Fetches the `planet` demo's maps (#220, D-056) into assets/planets/, which git ignores, checks
# each download's size and SHA-256, and converts them through Blender into what
# forge_terrain::planet reads:
#
#   tools/fetch-planets.sh            the Earth and the Moon
#   tools/fetch-planets.sh moon       only the Moon (about 95 MB; the Earth is 466 MB)
#
# - The Earth: NOAA NCEI's ETOPO 2022 global relief at 60 arc-seconds, ice surface (public
#   domain), to `earth/etopo-60s.i16`.
# - The Moon: NASA's CGI Moon Kit (https://svs.gsfc.nasa.gov/4720; credit "NASA's Scientific
#   Visualization Studio"): its elevation at 16 samples a degree to `moon/moon-ldem16.i16`, its
#   2025 colour map at 4096 × 2048 to `moon/moon-colour-4k.png`.
#
# A file already there with the right hash is kept, and a conversion already made is not made
# again. Blender is found on the PATH, else in its default Windows place (BLENDER overrides).
set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/assets/planets
bodies=("$@")
[ ${#bodies[@]} -gt 0 ] || bodies=(earth moon)

blender=${BLENDER:-}
if [ -z "$blender" ]; then
  if command -v blender > /dev/null; then
    blender=blender
  else
    blender=$(ls -d "/c/Program Files/Blender Foundation"/Blender*/blender.exe 2> /dev/null | tail -1)
  fi
fi
[ -n "$blender" ] || { echo "Blender not found: set BLENDER" >&2; exit 2; }

# fetch BODY FILE URL BYTES SHA256: downloads FILE into BODY's folder unless it is there.
fetch() {
  local body=$1 file=$2 url=$3 bytes=$4 sha=$5
  local path=$out/$body/$file
  mkdir -p "$out/$body"
  if [ -f "$path" ] && [ "$(sha256sum "$path" | cut -d' ' -f1)" = "$sha" ]; then
    echo "   kept $body/$file"
    return
  fi
  echo "   fetching $body/$file ($((bytes / 1000000)) MB)"
  curl -sfL --retry 3 -o "$path.part" "$url"
  local got
  got=$(stat -c %s "$path.part")
  if [ "$got" != "$bytes" ] || [ "$(sha256sum "$path.part" | cut -d' ' -f1)" != "$sha" ]; then
    rm -f "$path.part"
    echo "   $body/$file: $got bytes or its hash differ from the pinned ones" >&2
    exit 1
  fi
  mv "$path.part" "$path"
}

# convert SCRIPT SRC DST ARGS...: runs a Blender conversion unless DST is newer than SRC.
convert() {
  local script=$1 src=$2 dst=$3
  shift 3
  if [ -f "$dst" ] && [ "$dst" -nt "$src" ]; then
    echo "   kept $(basename "$dst")"
    return
  fi
  echo "   converting to $(basename "$dst")"
  # Blender wants Windows paths on Windows.
  local win_src=$src win_dst=$dst
  if command -v cygpath > /dev/null; then
    win_src=$(cygpath -w "$src")
    win_dst=$(cygpath -w "$dst")
  fi
  "$blender" --background --factory-startup --python "$here/assets/blender/$script" -- \
    "$win_src" "$win_dst" "$@" | grep -E "^planet " || true
  [ -f "$dst" ] || { echo "   the conversion to $dst failed" >&2; exit 1; }
}

nasa=https://svs.gsfc.nasa.gov/vis/a000000/a004700/a004720
for body in "${bodies[@]}"; do
  echo "== $body"
  case $body in
    earth)
      fetch earth ETOPO_2022_v1_60s_N90W180_surface.tif \
        https://www.ngdc.noaa.gov/mgg/global/relief/ETOPO2022/data/60s/60s_surface_elev_gtif/ETOPO_2022_v1_60s_N90W180_surface.tif \
        465969062 9d27d4b8ea8e76977e2988bca667d7c8fa68b927355feffcddd6b4875a7fd08e
      convert planet_elevation.py "$out/earth/ETOPO_2022_v1_60s_N90W180_surface.tif" \
        "$out/earth/etopo-60s.i16" 1 0
      ;;
    moon)
      fetch moon ldem_16_uint.tif "$nasa/ldem_16_uint.tif" \
        33201026 45a2b32d56e81ed30db07fead8abc842b249b6511219d9ca2c53f81bc2dc5d62
      fetch moon lroc_color_16bit_srgb_4k.tif "$nasa/lroc_color_16bit_srgb_4k.tif" \
        61891324 9731fa8af425b6c2f88f277ecca82bf8c603f3743894f64ed7b25c5bfefa22ff
      convert planet_elevation.py "$out/moon/ldem_16_uint.tif" "$out/moon/moon-ldem16.i16" \
        32767.5 -10000
      convert planet_colour.py "$out/moon/lroc_color_16bit_srgb_4k.tif" \
        "$out/moon/moon-colour-4k.png"
      ;;
    *) echo "unknown body $body: earth or moon" >&2; exit 2 ;;
  esac
done
