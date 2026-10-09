# A planet's elevation map for the `planet` demo (#220, D-056), from a downloaded TIFF, through
# Blender (no TIFF reader in the engine's dependencies):
#
#   blender --background --factory-startup --python assets/blender/planet_elevation.py -- \
#       SRC DST SCALE OFFSET
#
# SRC is an equirectangular map of the whole sphere, the north row first, longitude from -180°
# (tools/fetch-planets.sh fetches them into assets/planets/, which git ignores):
#   - the Earth: NOAA's ETOPO 2022, 60 arc-seconds, ice surface (float metres over the geoid):
#     SCALE 1, OFFSET 0;
#   - the Moon: NASA's CGI Moon Kit `ldem_16_uint.tif` (half-metres over 1 727 400 m), which
#     Blender reads as 0..1: SCALE 32767.5 (65535 / 2), OFFSET -10000 (to the mean radius,
#     1 737 400 m).
# Blender stores 16-bit and float TIFFs as float pixels; each is read as `value * SCALE + OFFSET`
# metres. DST is the same grid as little-endian `i16` metres, the north row first, which
# `forge_terrain::planet` reads.

import sys

import bpy
import numpy as np

src, dst, scale, offset = sys.argv[sys.argv.index("--") + 1:]
scale, offset = float(scale), float(offset)
img = bpy.data.images.load(src)
# The values as stored, no colour transform.
img.colorspace_settings.name = "Non-Color"
w, h = img.size
px = np.empty(w * h * 4, dtype=np.float32)
img.pixels.foreach_get(px)
# Blender's rows run bottom-up: flipped, the north row goes out first.
metres = px[0::4].reshape(h, w)[::-1] * scale + offset
grid = np.clip(np.rint(metres), -32768, 32767).astype("<i2")
grid.tofile(dst)
print("planet elevation", w, h, "metres", float(metres.min()), float(metres.max()))
