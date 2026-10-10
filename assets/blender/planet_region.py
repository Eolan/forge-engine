# A region's elevation for the `planet` demo (#220), from Copernicus DEM GLO-90 tiles (one
# degree each, 1 200 × 1 200 samples of float metres over the EGM2008 geoid below 50° of
# latitude), through Blender (no TIFF reader in the engine's dependencies):
#
#   blender --background --factory-startup --python assets/blender/planet_region.py -- \
#       SRC_DIR DST WEST EAST SOUTH NORTH
#
# WEST EAST SOUTH NORTH are whole degrees: the tiles `Copernicus_DSM_COG_30_N<lat>_00_E<lon>_00_DEM`
# from SOUTH to NORTH - 1 and WEST to EAST - 1 in SRC_DIR (tools/fetch-planets.sh fetches them).
# A tile that is not there is open sea: its samples are left as no data (-32768), where the
# engine reads the global map instead.
#
# Each tile is placed by its own GeoTIFF tags (its tie point, pixel scale and raster type), so
# the mosaic's samples sit where the tiles' do. DST is little-endian `i16` metres, the north row
# first, west to east. The script prints the bounds of the samples' cells, west east south north
# in degrees, for the world file's `[[map.regions]]`.

import os
import struct
import sys

import bpy
import numpy as np

NO_DATA = -32768
SAMPLES = 1200


def geo_tags(path):
    """The tie point (x, y), the pixel scale (sx, sy) and whether the tie point is a pixel's
    corner (PixelIsArea) rather than its centre, from a little-endian classic TIFF's IFD0."""
    with open(path, "rb") as f:
        data = f.read(1 << 20)
    if data[:4] != b"II*\x00":
        raise SystemExit(f"{path}: not a little-endian classic TIFF")
    (ifd,) = struct.unpack_from("<I", data, 4)
    (count,) = struct.unpack_from("<H", data, ifd)
    sizes = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 12: 8, 16: 8}
    tags = {}
    for k in range(count):
        tag, kind, n, value = struct.unpack_from("<HHII", data, ifd + 2 + 12 * k)
        width = sizes.get(kind, 1) * n
        at = ifd + 2 + 12 * k + 8 if width <= 4 else value
        if kind == 12:
            tags[tag] = struct.unpack_from(f"<{n}d", data, at)
        elif kind == 3:
            tags[tag] = struct.unpack_from(f"<{n}H", data, at)
    tie, scale, keys = tags[33922], tags[33550], tags[34735]
    area = True
    for k in range(keys[3]):
        key, _, _, value = keys[4 + 4 * k : 8 + 4 * k]
        if key == 1025:  # GTRasterTypeGeoKey: 1 PixelIsArea, 2 PixelIsPoint
            area = value == 1
    return (tie[3], tie[4]), (scale[0], scale[1]), area


src, dst, west, east, south, north = sys.argv[sys.argv.index("--") + 1 :]
west, east, south, north = int(west), int(east), int(south), int(north)
width, height = (east - west) * SAMPLES, (north - south) * SAMPLES
grid = np.full((height, width), NO_DATA, dtype="<i2")
origin = None
found = 0
for lat in range(south, north):
    for lon in range(west, east):
        name = f"Copernicus_DSM_COG_30_N{lat:02d}_00_E{lon:03d}_00_DEM"
        path = os.path.join(src, name + ".tif")
        if not os.path.exists(path):
            continue
        (x, y), (sx, sy), area = geo_tags(path)
        # The tile's first sample's centre.
        cx, cy = (x + 0.5 * sx, y - 0.5 * sy) if area else (x, y)
        img = bpy.data.images.load(path)
        img.colorspace_settings.name = "Non-Color"
        w, h = img.size
        if (w, h) != (SAMPLES, SAMPLES):
            raise SystemExit(f"{name}: {w} × {h}, not {SAMPLES} × {SAMPLES}")
        px = np.empty(w * h * 4, dtype=np.float32)
        img.pixels.foreach_get(px)
        bpy.data.images.remove(img)
        # Blender's rows run bottom-up: flipped, the north row first.
        metres = px[0::4].reshape(h, w)[::-1]
        if origin is None:
            # The mosaic's first sample's centre, from this tile's: its samples on the tiles'.
            origin = (cx - (lon - west) * SAMPLES * sx, cy + (north - 1 - lat) * SAMPLES * sy)
        col = (cx - origin[0]) / sx
        row = (origin[1] - cy) / sy
        if abs(col - round(col)) > 1e-3 or abs(row - round(row)) > 1e-3:
            raise SystemExit(f"{name}: its samples fall between the mosaic's ({col}, {row})")
        col, row = int(round(col)), int(round(row))
        grid[row : row + h, col : col + w] = np.clip(np.rint(metres), -32767, 32767)
        found += 1
        print("planet region tile", name, float(metres.min()), float(metres.max()))
if origin is None:
    raise SystemExit(f"no tiles in {src}")
grid.tofile(dst)
step = 1.0 / SAMPLES
# The bounds of the samples' cells: half a sample beyond the first and last centres.
bounds = (
    origin[0] - 0.5 * step,
    origin[0] + (width - 0.5) * step,
    origin[1] - (height - 0.5) * step,
    origin[1] + 0.5 * step,
)
print("planet region", width, height, "tiles", found, "bounds", " ".join(f"{b:.6f}" for b in bounds))
