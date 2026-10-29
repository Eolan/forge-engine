# The material test chart (D-083 step 2): one glTF scene and its skies, for Forge
# (`physics-lab --lab chart`) and Falcor to render alike and compare in linear light.
#
#   blender -b --factory-startup --python tools/material-chart.py -- OUT_DIR
#
# Writes into OUT_DIR (assets/external/chart/, git-ignored):
# - chart.gltf (+ .bin): a grey ground; four rows of spheres, 0.4 m, at seven roughnesses from
#   0.05 to 1 (grey and red dielectrics, silver and gold metals); a row of flat tiles in front:
#   white and grey diffuse (the references), a smooth dielectric and a rough metal. Factors only,
#   no textures. Y up, metres: the spheres on x -3..3, z 0..3.6.
# - sky-<from>-<elevation>.hdr: Falcor's latitude-longitude layout, 2048 x 1024; a uniform sky of
#   radiance 1 above the horizon, black below, and a sun disc of 0.5° radius whose irradiance on
#   a surface facing it is 10. From "side" the sun shines towards +x, from "front" towards +z
#   (a camera looking down +z faces it); elevations 60, 30 and 10 degrees.
# - chart.pyscene: Falcor's scene file, the sky by FORGE_CHART_SKY's name at intensity 1.

import math
import os
import sys

import bpy
import numpy as np

out = sys.argv[sys.argv.index("--") + 1]
os.makedirs(out, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)


def material(name, color, metallic, roughness):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    p = m.node_tree.nodes["Principled BSDF"]
    p.inputs["Base Color"].default_value = (*color, 1.0)
    p.inputs["Metallic"].default_value = metallic
    p.inputs["Roughness"].default_value = roughness
    return m


def place(obj, x, y_up, z):
    # glTF's (x, y up, z) from Blender's Z-up: Blender (x, -z, y).
    obj.location = (x, -z, y_up)


# The ground: 40 m square, 18 % grey, matte.
bpy.ops.mesh.primitive_plane_add(size=40.0)
ground = bpy.context.active_object
ground.name = "chart_ground"
ground.data.materials.append(material("chart_ground", (0.18, 0.18, 0.18), 0.0, 1.0))

ROUGHNESS = [0.05, 0.2, 0.35, 0.5, 0.65, 0.8, 1.0]
ROWS = [
    ("grey", (0.5, 0.5, 0.5), 0.0),
    ("red", (0.8, 0.05, 0.05), 0.0),
    ("silver", (0.95, 0.95, 0.95), 1.0),
    ("gold", (1.0, 0.77, 0.34), 1.0),
]
for r, (name, color, metallic) in enumerate(ROWS):
    for c, rough in enumerate(ROUGHNESS):
        bpy.ops.mesh.primitive_uv_sphere_add(segments=64, ring_count=32, radius=0.4)
        s = bpy.context.active_object
        bpy.ops.object.shade_smooth()
        s.name = f"chart_{name}_r{int(rough * 100):03d}"
        s.data.materials.append(material(s.name, color, metallic, rough))
        place(s, -3.0 + c, 0.4, 1.2 * r)

TILES = [
    ("white", (0.8, 0.8, 0.8), 0.0, 1.0),
    ("grey18", (0.18, 0.18, 0.18), 0.0, 1.0),
    ("smooth", (0.5, 0.5, 0.5), 0.0, 0.05),
    ("metal", (0.95, 0.95, 0.95), 1.0, 0.3),
]
for t, (name, color, metallic, rough) in enumerate(TILES):
    bpy.ops.mesh.primitive_plane_add(size=0.8)
    p = bpy.context.active_object
    p.name = f"chart_tile_{name}"
    p.data.materials.append(material(p.name, color, metallic, rough))
    place(p, -1.5 + t, 0.002, -1.4)

bpy.ops.export_scene.gltf(
    filepath=os.path.join(out, "chart.gltf"),
    export_format="GLTF_SEPARATE",
    export_lights=False,
    export_cameras=False,
    export_yup=True,
)

def write_hdr(path, grey):
    # A Radiance RGBE file, rows top to bottom, uncompressed: Blender's writer cut the sun's
    # 41,700 to 90.5.
    mantissa, exponent = np.frexp(grey)
    m = np.clip(mantissa * 256.0, 0, 255).astype(np.uint8)
    e = np.where(grey > 1e-32, exponent + 128, 0).astype(np.uint8)
    m = np.where(grey > 1e-32, m, 0).astype(np.uint8)
    rgbe = np.stack([m, m, m, e], axis=-1)
    with open(path, "wb") as f:
        header = f"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {grey.shape[0]} +X {grey.shape[1]}\n"
        f.write(header.encode())
        f.write(rgbe.tobytes())


# The skies, in Falcor's layout: u = (1 + atan2(x, -z) / pi) / 2, v = acos(y) / pi.
W, H = 2048, 1024
SUN_RADIUS = math.radians(0.5)
SUN_IRRADIANCE = 10.0
u = (np.arange(W) + 0.5) / W
v = (np.arange(H) + 0.5) / H
phi = (2.0 * u - 1.0) * math.pi
theta = v * math.pi
x = np.sin(phi)[None, :] * np.sin(theta)[:, None]
y = np.cos(theta)[:, None] * np.ones((1, W))
z = -np.cos(phi)[None, :] * np.sin(theta)[:, None]
for side, toward in (("side", (1.0, 0.0)), ("front", (0.0, 1.0))):
    for elevation in (60, 30, 10):
        e = math.radians(elevation)
        sun = (toward[0] * math.cos(e), math.sin(e), toward[1] * math.cos(e))
        radiance = np.where(y > 0.0, 1.0, 0.0)
        cosine = x * sun[0] + y * sun[1] + z * sun[2]
        disc = cosine > math.cos(SUN_RADIUS)
        # The disc's texels share the irradiance by their solid angle, so it sums to exactly 10.
        solid = (2.0 * math.pi / W) * (math.pi / H) * np.sin(theta)[:, None] * np.ones((1, W))
        radiance = np.where(disc, SUN_IRRADIANCE / (solid * disc).sum(), radiance)
        write_hdr(os.path.join(out, f"sky-{side}-{elevation}.hdr"), radiance)

with open(os.path.join(out, "chart.pyscene"), "w") as f:
    f.write(
        "# The material test chart (tools/material-chart.py) under one of its skies.\n"
        "import os\n"
        'sceneBuilder.importScene("chart.gltf")\n'
        'sceneBuilder.envMap = EnvMap(os.environ.get("FORGE_CHART_SKY", "sky-side-30.hdr"))\n'
        "sceneBuilder.envMap.intensity = 1\n"
    )
print("CHART written to", out)
