# Blender script (D-083): an FBX scene to glTF for Forge, its textures left where they are.
#
#   blender -b --factory-startup --python tools/fbx-to-gltf.py -- IN.fbx OUT.gltf
#
# Writes OUT.gltf and OUT.bin (the geometry, the nodes, the materials' factors and the
# cameras; no lights, Forge lights the scene itself, and no images) and OUT.textures.json,
# which names each material's texture files by their role (base colour, normal, specular,
# emissive), relative to the FBX's folder: Forge reads them as they are (block-compressed DDS
# with their mips) instead of the PNGs Blender would write. When the scene has an animated
# camera, OUT.camera.json holds its path, one [x, y, z, qx, qy, qz, qw] a frame in glTF's frame
# (+Y up, metres).
#
# Keep OUT out of a folder named glTF under assets/external/: the external models' test loads
# every model there (crates/forge-geom/tests/external_models.rs).
import bpy, json, os, sys

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
print(f"fbx-to-gltf: {len(textures)} materials, camera {'yes' if camera else 'no'} -> {out}")
