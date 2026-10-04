# The Moon's albedo for the real sky (D-046, #164), from NASA's CGI Moon Kit, through Blender:
#
#   blender --background --factory-startup --python assets/blender/moon_albedo.py -- \
#       lroc_color_2k.jpg assets/sky/moon-albedo-512x256.r8
#
# The input is the kit's 2025 colour map (https://svs.gsfc.nasa.gov/4720, lroc_color_2k.jpg,
# 2048 x 1024, equirectangular, longitude 0 in the middle; credit: NASA's Scientific
# Visualization Studio). The output is its linear luminance at 512 x 256, one byte a texel, the
# top row first, scaled so the brightest texel is 255; forge_render::night reads it and scales
# it so its mean is 1 (the disc keeps the Moon's measured illuminance).

import sys

import bpy

src, dst = sys.argv[sys.argv.index("--") + 1:]
img = bpy.data.images.load(src)
# The bytes as they are: decoded from sRGB below.
img.colorspace_settings.name = "Non-Color"
img.scale(512, 256)
w, h = img.size
px = list(img.pixels)


def linear(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


values = []
# Blender's rows run bottom-up: the top row goes out first.
for y in range(h - 1, -1, -1):
    for x in range(w):
        i = (y * w + x) * 4
        r, g, b = linear(px[i]), linear(px[i + 1]), linear(px[i + 2])
        values.append(0.2126 * r + 0.7152 * g + 0.0722 * b)
top = max(values)
with open(dst, "wb") as f:
    f.write(bytes(min(255, round(255 * v / top)) for v in values))
print("moon albedo", w, h, "mean over max", sum(values) / len(values) / top)
