# Blender script (D-083): an FBX scene to glTF for Forge, its textures left where they are.
#
#   blender -b --factory-startup --python tools/fbx-to-gltf.py -- IN.fbx OUT.gltf
#
# Writes OUT.gltf and OUT.bin (the geometry, the nodes, the materials' factors and the
# cameras; no lights, Forge lights the scene itself, and no images) and OUT.textures.json,
# which names each material's texture files by their role (base colour, normal, specular,
# emissive), relative to the FBX's folder: Forge reads them as they are (block-compressed DDS
# with their mips) instead of the PNGs Blender would write. OUT.lights.json holds the lights
# the glTF leaves out (a reference matches their sun). When the scene has an animated camera,
# OUT.camera.json holds its path, one [x, y, z, qx, qy, qz, qw] a frame in glTF's frame (+Y
# up, metres).
#
# Keep OUT out of a folder named glTF under assets/external/: the external models' test loads
# every model there (crates/forge-geom/tests/external_models.rs).
import bpy, json, os, sys
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
src, out = os.path.abspath(argv[0]), os.path.abspath(argv[1])
base = os.path.dirname(src)

for o in list(bpy.data.objects):
    bpy.data.objects.remove(o, do_unlink=True)
bpy.ops.import_scene.fbx(filepath=src)

# Each material's texture files, by the role their names end in (Bistro's convention:
# `<Material>_BaseColor.dds`, `_Normal`, `_Specular` = occlusion, roughness, metalness in R, G,
# B, `_Emissive`).
ROLES = {"_basecolor": "base_color", "_normal": "normal", "_specular": "specular",
         "_emissive": "emissive"}
textures = {}
for m in bpy.data.materials:
    roles = {}
    if m.use_nodes:
        for n in m.node_tree.nodes:
            if n.type != "TEX_IMAGE" or not n.image:
                continue
            path = bpy.path.abspath(n.image.filepath)
            stem = os.path.splitext(os.path.basename(path))[0].lower()
            for suffix, role in ROLES.items():
                if stem.endswith(suffix):
                    roles[role] = os.path.relpath(path, base).replace("\\", "/")
    textures[m.name] = roles

# The camera's path, before the export moves anything.
camera = None
scene = bpy.context.scene
cams = [o for o in bpy.data.objects if o.type == "CAMERA" and o.animation_data
        and o.animation_data.action]
if cams:
    cam = cams[0]
    first, last = (int(f) for f in cam.animation_data.action.frame_range)
    frames = []
    for f in range(first, last + 1):
        scene.frame_set(f)
        m = cam.matrix_world
        t, q = m.translation, m.to_quaternion()
        # Blender +Z up to glTF +Y up: (x, y, z) -> (x, z, -y); the rotation the same way.
        frames.append([t.x, t.z, -t.y, q.x, q.z, -q.y, q.w])
    camera = {"name": cam.name, "fps": scene.render.fps, "lens_mm": cam.data.lens,
              "sensor_mm": cam.data.sensor_width, "frames": frames}

# The scene's lights, which the glTF leaves out: where its authors put the sun and how bright,
# for a reference to match. Each light's type, colour, Blender energy (W/m² for a sun, W
# otherwise), position and the direction it shines (its local −Z), in glTF's frame.
lights = []
for o in bpy.data.objects:
    if o.type != "LIGHT":
        continue
    m = o.matrix_world
    t = m.translation
    d = (m.to_3x3() @ Vector((0.0, 0.0, -1.0))).normalized()
    lights.append({"name": o.name, "type": o.data.type, "color": list(o.data.color),
                   "energy": o.data.energy, "position": [t.x, t.z, -t.y],
                   "direction": [d.x, d.z, -d.y]})

bpy.ops.export_scene.gltf(
    filepath=out,
    export_format="GLTF_SEPARATE",
    export_image_format="NONE",
    export_materials="EXPORT",
    export_cameras=True,
    export_lights=False,
    export_yup=True,
    export_animations=False,
)
stem = os.path.splitext(out)[0]
with open(stem + ".textures.json", "w") as f:
    json.dump(textures, f, indent=1, sort_keys=True)
if camera:
    with open(stem + ".camera.json", "w") as f:
        json.dump(camera, f)
with open(stem + ".lights.json", "w") as f:
    json.dump(lights, f, indent=1)
print(f"fbx-to-gltf: {len(textures)} materials, {len(lights)} lights, camera {'yes' if camera else 'no'} -> {out}")
