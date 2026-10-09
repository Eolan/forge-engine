# A planet's sea mask for the `planet` demo (#220), from its elevation grid, through Blender:
#
#   blender --background --factory-startup --python assets/blender/planet_sea_mask.py -- \
#       SRC DST WIDTH HEIGHT DST_WIDTH DST_HEIGHT
#
# SRC is the planet's elevation as `planet_elevation.py` writes it: WIDTH × HEIGHT little-endian
# `i16` metres, the north row first, longitude from -180°. DST is a grey PNG of DST_WIDTH ×
# DST_HEIGHT, the same projection: white where the ground lies under the sea's level, its edges
# the resampling's blend. The shader reads it for the sea where a pixel spans more than the
# ground's own triangles show, so a coarse tile's coast follows the map, not its triangles.

import sys

import bpy
import numpy as np

args = sys.argv[sys.argv.index("--") + 1:]
src, dst, width, height, dst_width, dst_height = args
width, height, dst_width, dst_height = int(width), int(height), int(dst_width), int(dst_height)
heights = np.fromfile(src, dtype="<i2").reshape(height, width)
sea = (heights < 0).astype(np.float32)
# Blender's rows run bottom-up.
rgba = np.empty((height, width, 4), dtype=np.float32)
rgba[..., 0] = rgba[..., 1] = rgba[..., 2] = sea[::-1]
rgba[..., 3] = 1.0
img = bpy.data.images.new("sea", width, height, alpha=False, float_buffer=True)
img.colorspace_settings.name = "Non-Color"
img.pixels.foreach_set(rgba.ravel())
img.scale(dst_width, dst_height)
scene = bpy.context.scene
settings = scene.render.image_settings
settings.file_format = "PNG"
settings.color_depth = "8"
settings.color_mode = "BW"
scene.view_settings.view_transform = "Standard"
scene.view_settings.look = "None"
scene.display_settings.display_device = "sRGB"
img.save_render(dst, scene=scene)
print("planet sea mask", dst_width, dst_height, "sea share", float(sea.mean()))
