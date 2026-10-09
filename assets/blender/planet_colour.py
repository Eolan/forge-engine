# A body's colour map for the `planet` demo (#220), from a downloaded image, through Blender (no
# TIFF reader in the engine's dependencies, and Blender resamples):
#
#   blender --background --factory-startup --python assets/blender/planet_colour.py -- \
#       SRC DST [WIDTH HEIGHT]
#
# SRC is an equirectangular colour map of the whole body, the north row first, longitude from
# -180°:
#   - the Moon's from NASA's CGI Moon Kit (`lroc_color_16bit_srgb_4k.tif`, 16-bit sRGB; credit:
#     NASA's Scientific Visualization Studio), kept at its 4096 × 2048;
#   - the Earth's from NASA's Blue Marble Next Generation, July 2004, without its relief shaded
#     (`world.200407.3x21600x10800.jpg`; credit: NASA Earth Observatory), resampled to
#     16384 × 8192: a side a power of two, as the engine's textures take, and no wider than the
#     16 384 AMD's GPUs allow.
# DST is the same image as an 8-bit sRGB PNG or JPEG (by its extension), which
# `forge_render::textures::decode_image` reads with its mips.

import sys

import bpy

args = sys.argv[sys.argv.index("--") + 1:]
src, dst = args[0], args[1]
img = bpy.data.images.load(src)
# Loaded on first use: read one pixel so it is.
img.pixels[0]
if len(args) >= 4:
    img.scale(int(args[2]), int(args[3]))
jpeg = dst.lower().endswith((".jpg", ".jpeg"))
img.filepath_raw = dst
img.file_format = "JPEG" if jpeg else "PNG"
scene = bpy.context.scene
settings = scene.render.image_settings
settings.file_format = "JPEG" if jpeg else "PNG"
settings.color_mode = "RGB"
if jpeg:
    settings.quality = 92
else:
    settings.color_depth = "8"
# As stored: no view transform (Blender's default, AgX, would lighten and grey it).
scene.view_settings.view_transform = "Standard"
scene.view_settings.look = "None"
scene.view_settings.exposure = 0.0
scene.view_settings.gamma = 1.0
img.save_render(dst, scene=scene)
print("planet colour", img.size[0], img.size[1])
