#!/usr/bin/env bash
# Fetches the `planet` demo's maps (#220, D-056) into assets/planets/, which git ignores, checks
# each download's size and SHA-256, and converts them through Blender into what
# forge_terrain::planet reads:
#
#   tools/fetch-planets.sh            the Earth, the Moon and the sky
#   tools/fetch-planets.sh moon       only the Moon (about 95 MB; the Earth is 584 MB, the sky 131 MB)
#
# - The Earth: NOAA NCEI's ETOPO 2022 global relief at 60 arc-seconds, ice surface (public
#   domain), to `earth/etopo-60s.i16`; NASA's Blue Marble Next Generation for July 2004 without
#   its relief shaded (credit "NASA Earth Observatory"), to `earth/earth-colour-16k.jpg`; the sea
#   from the elevation, to `earth/earth-sea-8k.png`. The tour's region at 90 m: 25 tiles of the
#   Copernicus DEM GLO-90 (97 MB; AWS's open data registry, free under the Copernicus licence;
#   the mosaic is modified data: "produced using Copernicus WorldDEM-90 © DLR e.V. 2010-2014 and
#   © Airbus Defence and Space GmbH 2014-2018 provided under COPERNICUS by the European Union and
#   ESA; all rights reserved"), to `earth/glo90-alps.i16`.
# - The Moon: NASA's CGI Moon Kit (https://svs.gsfc.nasa.gov/4720; credit "NASA's Scientific
#   Visualization Studio"): its elevation at 16 samples a degree to `moon/moon-ldem16.i16`, its
#   2025 colour map at 4096 × 2048 to `moon/moon-colour-4k.png`.
# - The sky: NASA's Deep Star Maps 2020 at 8K (https://svs.gsfc.nasa.gov/4851; from ESA's Gaia and
#   Hipparcos; credit "NASA/Goddard Space Flight Center Scientific Visualization Studio" and
#   "ESA/Gaia/DPAC"), to `sky/stars-8k.png` (its values are a display's, 0 to 1: sRGB keeps them).
#
# A file already there with the right hash is kept, and a conversion already made is not made
# again. Blender is found on the PATH, else in its default Windows place (BLENDER overrides).
set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/assets/planets
bodies=("$@")
[ ${#bodies[@]} -gt 0 ] || bodies=(earth moon sky)

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
      fetch earth world.200407.3x21600x10800.jpg \
        https://assets.science.nasa.gov/content/dam/science/esd/eo/images/bmng/bmng-base/july/world.200407.3x21600x10800.jpg \
        21125326 dea8b4dc8a4f93f5f8bce0c8c85a508a178e7901e9ed8e6bf86e6ce7ef6d61e2
      convert planet_colour.py "$out/earth/world.200407.3x21600x10800.jpg" \
        "$out/earth/earth-colour-16k.jpg" 16384 8192
      convert planet_sea_mask.py "$out/earth/etopo-60s.i16" "$out/earth/earth-sea-8k.png" \
        21600 10800 8192 4096
      # The tour's region at 90 m (#220): Copernicus DEM GLO-90 from 41° to 47° N and 5° to 10°
      # E, the Alps, the Côte d'Azur and Corsica, one tile a degree (open sea has none).
      glo90=https://copernicus-dem-90m.s3.amazonaws.com
      while read -r tile bytes sha; do
        name=Copernicus_DSM_COG_30_${tile}_DEM
        fetch earth/glo90 "$name.tif" "$glo90/$name/$name.tif" "$bytes" "$sha"
      done << 'EOF'
N41_00_E008_00 867488 9ce7adec542c3438b4b9f46fdff708b218a346a2990863301fd1ad5ad0561d3e
N41_00_E009_00 1953182 f95ddbdf8c0ccb94975935206ff2abbd2e9a16f8565f68d1139574ab3136b601
N42_00_E006_00 57893 0dc14b91f8e9911c37f2aa7fbbcc091f3d0d5bcdafad86cfb8e669a16d453da5
N42_00_E008_00 1313384 f689701c1c6866b3cfea3da24eb0139944d4492381ba9a43e05964e94d30f5c3
N42_00_E009_00 2434748 8363b785af7d2570903f21cb28c52427f4e7495baac76ba55653609a92596d1e
N43_00_E005_00 4276141 d0de5ffc04334b36626a3363f8b90e0c4eeedda7474e974b648a6717ad0aacba
N43_00_E006_00 4294068 738bacdf7d2f4bb2b54fb6f292bd4d5d4f2e7c8d16450645ac8ed3d16b00ef69
N43_00_E007_00 1585352 9fb02abe4417bcf48ac96e8f537974beeb36b4a1cbd3ce04b681cd1b077b068d
N43_00_E008_00 152056 8a77a66af8dc2d1dd3ff9fe14f51115338a2338f415536819d4736c4fdd040bc
N43_00_E009_00 76087 dafd5832edecb475ef532fb49092cd79542078a364bb556e4f05f5ec5c006c44
N44_00_E005_00 5360250 a7a7a0872e39ca16a70ad8b41522d8c86179cf9955502b336de7f9564909a5a0
N44_00_E006_00 5236643 c92a34d911fbde3874ffb83e7d1d467fe1ce98489b8c73d70cfc897835b70c09
N44_00_E007_00 5196747 cd6c0889a74430c09c40e5bd8df27af8d50bb9cf27f3ebd254ce2e6f4f6c55b6
N44_00_E008_00 4255878 d132d66094bba8584310d6767c5a97b60826407c04ab8b4f4e4758267f43f1af
N44_00_E009_00 4405659 6d6f05365b9a81b9ee70181263da57d7d3cf4418e73f027eff206d438ded39ae
N45_00_E005_00 5334594 a739af6d34f4412be99de54fccb0ed756ac8ffaeed15cbf0b5c4d1f1c6052b0a
N45_00_E006_00 5249745 a002f90ad4b187486930ca502cc8ad67bcf6c65f973e5920702b6d068fc876ce
N45_00_E007_00 5244538 a581f2d9539b30e64dce807b9105752689fba3261e0c7ae62602d7f24f1f0d43
N45_00_E008_00 5138554 17ef05156ba29d063dfdfc75f015417099cf515e1d1cd0c14fa39c5aa5dfda55
N45_00_E009_00 5212138 c897ede243e6ed465f1a5be9b43a3f98ae6a06a45dfbcccab2ec650269a634de
N46_00_E005_00 5302301 85cf6a624fbf61a3e20761ceec65989947b1268d60300955e656aa1095c3ee59
N46_00_E006_00 4712364 1a790f6987d5644281c76a92bad2d1997e08170c9912832443f99d1b5ad24bcf
N46_00_E007_00 5157813 3528ab5ba11ec63b6fd3dff91736911e69a0af0d87e50c7b61bc259deab9e732
N46_00_E008_00 5230979 27f1c6555357b4933eaeda91b39bdfab79a2d4688c6562ca26af374f6e3ab4fc
N46_00_E009_00 5225729 6860c0b93a271f3b702a4a16aa885ba19c4907e808c56f94a620c6a657aa5f18
EOF
      convert planet_region.py "$out/earth/glo90" "$out/earth/glo90-alps.i16" 5 10 41 47
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
    sky)
      fetch sky starmap_2020_8k.exr \
        https://svs.gsfc.nasa.gov/vis/a000000/a004800/a004851/starmap_2020_8k.exr \
        130530278 dc6c4f413e85707a29a25a9451148154554ecca2c996f84fa8f47b65ef9ff7c4
      convert planet_colour.py "$out/sky/starmap_2020_8k.exr" "$out/sky/stars-8k.png"
      ;;
    *) echo "unknown body $body: earth, moon or sky" >&2; exit 2 ;;
  esac
done
