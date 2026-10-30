# The measures behind tools/gi-compare.sh (D-083's calibration, D-088): linear images (Radiance
# .hdr or OpenEXR) compared in luminance, as Blender reads them.
#
#   blender -b --factory-startup --python tools/gi-measure.py -- tiles REF SCALE TEST...
#   blender -b --factory-startup --python tools/gi-measure.py -- tonemap OUT_DIR KEY NAME=PATH...
#   blender -b --factory-startup --python tools/gi-measure.py -- ratio OUT_DIR REF SCALE BLOCK NAME=PATH...
#   blender -b --factory-startup --python tools/gi-measure.py -- blank OUT_PNG LIKE_PNG
#
# - tiles: each TEST's luminance in 16 x 9 tiles against REF's times SCALE (Falcor's units to
#   Forge's cd/m²: the Bistro's 2,871, the chart's and the classroom's 12,800). One line each:
#   `TIL NAME |log2| MEAN signed MEAN frame xRATIO`, the mean |log2| of the tiles' ratios, their
#   signed mean and the whole frame's light over the reference's. A TEST written A::B is A's
#   tiles less B's (the sky alone as the whole less the sun alone; both renderers are linear in
#   their light). The tiles average out a path tracer's grain.
# - tonemap: images tone-mapped alike for a sheet. The first image's median luminance sets one
#   exposure for all, putting it at KEY (0.18); then Narkowicz's ACES fit and the sRGB curve,
#   written as OUT_DIR/NAME.png.
# - ratio: each image's luminance over REF's (times SCALE) in BLOCK x BLOCK blocks, as
#   OUT_DIR/NAME.png: white equal, red brighter, blue darker, full colour at one stop. The image
#   is scaled to REF's size first. Prints `RATIO NAME mean|log2| MEAN frame xRATIO`.
# - blank: a white PNG of LIKE_PNG's size (the reference's place among the maps).
#
# A PATH may end in *SCALE (its light times SCALE). The images are data: nothing in them runs.

import os
import sys

import bpy
import numpy as np

args = sys.argv[sys.argv.index("--") + 1:]


def split_scale(item):
    """PATH[*SCALE] -> (PATH, SCALE)."""
    path, scale = (item.rsplit("*", 1) + ["1"])[:2] if "*" in item else (item, "1")
    return path, float(scale)


def load(path, size=None):
    """An image's linear RGB (rows top to bottom), scaled to `size` when given."""
    im = bpy.data.images.load(path)
    if size is not None and tuple(im.size) != size:
        im.scale(*size)
    w, h = im.size
    px = np.empty(w * h * 4, dtype=np.float32)
    im.pixels.foreach_get(px)
    return px.reshape(h, w, 4)[::-1, :, :3].astype(np.float64), (w, h)


def luminance(rgb):
    return 0.2126 * rgb[..., 0] + 0.7152 * rgb[..., 1] + 0.0722 * rgb[..., 2]


def blocks(lum, rows, cols):
    h, w = lum.shape
    bh, bw = h // rows, w // cols
    return lum[: bh * rows, : bw * cols].reshape(rows, bh, cols, bw).mean(axis=(1, 3))


def save_png(rgb, path):
    """RGB already in display values, rows top to bottom."""
    h, w = rgb.shape[:2]
    im = bpy.data.images.new(os.path.basename(path), w, h, alpha=True)
    im.colorspace_settings.name = "Non-Color"
    px = np.concatenate([rgb[::-1], np.ones((h, w, 1))], axis=-1).astype(np.float32)
    im.pixels.foreach_set(px.ravel())
    im.filepath_raw = path
    im.file_format = "PNG"
    im.save()


def tiles(item):
    def one(p):
        path, scale = split_scale(p)
        return blocks(luminance(load(path)[0]) * scale, 9, 16)

    parts = item.split("::")
    t = one(parts[0])
    return t - one(parts[1]) if len(parts) > 1 else t


def name_of(item):
    return item.split("::")[0].replace("\\", "/").split("/")[-1].split("*")[0]


command = args[0]
if command == "tiles":
    ref = tiles(args[1]) * float(args[2])
    for item in args[3:]:
        t = tiles(item)
        r = np.log2(np.maximum(t, 1e-9) / np.maximum(ref, 1e-9))
        print(
            "TIL %-34s |log2| %.3f  signed %+.3f  frame x%.3f"
            % (name_of(item)[:34], np.mean(np.abs(r)), np.mean(r), t.mean() / ref.mean())
        )
elif command == "tonemap":
    out, key = args[1], float(args[2])
    exposure = None
    for item in args[3:]:
        name, rest = item.split("=", 1)
        path, scale = split_scale(rest)
        rgb = load(path)[0] * scale
        if exposure is None:
            exposure = key / max(np.median(luminance(rgb)), 1e-12)
            print("EXPOSURE", exposure)
        x = rgb * exposure
        x = np.clip((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0)
        x = np.where(x <= 0.0031308, 12.92 * x, 1.055 * np.power(x, 1 / 2.4) - 0.055)
        save_png(x, os.path.join(out, name + ".png"))
elif command == "ratio":
    out, ref_item, scale, block = args[1], args[2], float(args[3]), int(args[4])
    ref_rgb, size = load(split_scale(ref_item)[0])
    ref_lum = luminance(ref_rgb) * split_scale(ref_item)[1] * scale
    h, w = ref_lum.shape
    rows, cols = h // block, w // block
    ref = blocks(ref_lum, rows, cols)
    for item in args[5:]:
        name, rest = item.split("=", 1)
        path, tscale = split_scale(rest)
        test = blocks(luminance(load(path, size)[0]) * tscale, rows, cols)
        r = np.log2(np.maximum(test, 1e-9) / np.maximum(ref, 1e-9))
        print("RATIO %-12s mean|log2| %.3f  frame x%.3f" % (name, np.mean(np.abs(r)), test.mean() / ref.mean()))
        t = np.clip(r, -1.0, 1.0)
        rgb = np.ones(t.shape + (3,))
        rgb[..., 0] = np.where(t < 0, 1.0 + t, 1.0)
        rgb[..., 1] = 1.0 - np.abs(t)
        rgb[..., 2] = np.where(t > 0, 1.0 - t, 1.0)
        save_png(np.repeat(np.repeat(rgb, block, axis=0), block, axis=1), os.path.join(out, name + ".png"))
elif command == "blank":
    like = bpy.data.images.load(args[2])
    w, h = like.size
    save_png(np.ones((h, w, 3)), args[1])
else:
    raise SystemExit("gi-measure.py: tiles, tonemap, ratio or blank, not " + command)
