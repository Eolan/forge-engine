# The physics lab's flyer (Phase 3's step 7), a gull modelled and animated in Blender from code
# and exported as glTF:
#
#   blender --background --factory-startup --python assets/blender/bird.py -- OUT.glb
#
# Built as the creatures are (`skinned_creatures.py`, whose helpers it uses): one continuous
# body from overlapping shapes, voxel-remeshed and decimated, on an armature of seven bones
# weighted by Blender's bone heat: the body, the head, the tail, and each wing's arm and hand.
# Its wings span 1.3 m, its body is 0.6 m from beak to tail.
#
# Texture: white head, body and tail, pale grey wings above, black wing tips with a white spot,
# a yellow bill with a red spot, black eyes; baked by Cycles into 512 x 512 PNGs as the
# creatures' are.
#
# Clips, sampled at 24 frames a second and looping (last key = first):
#   bird-flap   1/3 s: the wings beating 0.75 rad up and down, twisting with the beat (the hands
#               most), the hands folding back on the
#               upstroke, the tail and the head
#               steadying;
#   bird-glide  2 s: the wings held out with a little dihedral, flexing, the head looking about.
#
# The bird's origin is its body's middle; it faces Blender's +Y (Forge's forward, -Z). Blender's
# +Z up becomes Forge's +Y.

import math
import os
import sys
import tempfile

import bpy
from mathutils import Vector

# The creatures' helpers, imported without leaving a bytecode cache in the repository.
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import skinned_creatures as sc  # noqa: E402


def feathers(eyes, bill, spot):
    """White below and on the head and tail, pale grey wings above, black tips with a white
    mirror, a yellow bill (`bill`, (centre, radius)) with a red spot (`spot`), black eyes."""

    def shade(n):
        coord = n.new("ShaderNodeTexCoord").outputs["Object"]
        xyz = n.new("ShaderNodeSeparateXYZ")
        n.link(coord, xyz.inputs["Vector"])
        x, y = xyz.outputs["X"], xyz.outputs["Y"]
        along = n.new("ShaderNodeAttribute", attribute_name="grain").outputs["Vector"]
        barbs = n.noise(along, (2.0, 60.0, 60.0), 6.0)
        span = n.math("ABSOLUTE", x)
        facing = n.new("ShaderNodeSeparateXYZ")
        n.link(n.new("ShaderNodeNewGeometry").outputs["Normal"], facing.inputs["Vector"])
        up = n.ramp(facing.outputs["Z"], -0.1, 0.4)
        white = n.lerp((0.78, 0.79, 0.8), (0.9, 0.9, 0.9), barbs)
        grey = n.lerp((0.36, 0.39, 0.43), (0.46, 0.49, 0.53), barbs)
        # The mantle and the wings' tops, from the shoulders out.
        mantle = n.math("MULTIPLY", up, n.math("MAXIMUM", n.ramp(span, 0.07, 0.12), n.math("MULTIPLY", n.ramp(y, 0.1, 0.05), n.ramp(y, -0.16, -0.12))))
        color = n.lerp(white, grey, mantle)
        # The tips, black on both faces, a white mirror near the end.
        tip = n.ramp(span, 0.5, 0.54)
        mirror = n.math("MULTIPLY", n.ramp(span, 0.58, 0.6), n.ramp(span, 0.64, 0.62))
        color = n.lerp(color, n.lerp((0.03, 0.03, 0.03), (0.07, 0.07, 0.07), barbs), n.math("MULTIPLY", tip, n.math("SUBTRACT", 1.0, mirror)))
        yellow = n.spheres(coord, [bill], 0.004)
        color = n.lerp(color, (0.85, 0.62, 0.08), yellow)
        color = n.lerp(color, (0.7, 0.08, 0.04), n.spheres(coord, [spot], 0.003))
        eye = n.spheres(coord, eyes, 0.002)
        color = n.lerp(color, (0.01, 0.01, 0.01), eye)
        roughness = n.lerp(n.lerp(0.7, 0.35, yellow), 0.08, eye)
        height = n.math("MULTIPLY", barbs, n.math("SUBTRACT", 1.0, n.math("MAXIMUM", eye, yellow)))
        return color, n.vmath("DOT_PRODUCT", roughness, (1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0)), height, 0.15

    return shade


def bird(folder):
    bm = sc.bmesh.new()
    # The body, the head and the bill, the tail: +y is the front.
    sc.capsule(bm, (0.0, -0.17, 0.0), (0.0, 0.11, 0.01), 0.065, scale=(0.85, 1.0))
    sc.blob(bm, (0.0, 0.17, 0.045), (0.04, 0.052, 0.042))
    sc.capsule(bm, (0.0, 0.21, 0.04), (0.0, 0.29, 0.03), 0.011)
    sc.capsule(bm, (0.0, -0.18, 0.005), (0.0, -0.3, 0.0), 0.045, scale=(1.5, 0.22))
    bones = [
        ("body", (0.0, -0.15, 0.0), (0.0, 0.11, 0.01), None),
        ("head", (0.0, 0.12, 0.02), (0.0, 0.29, 0.03), "body"),
        ("tail", (0.0, -0.17, 0.005), (0.0, -0.31, 0.0), "body"),
    ]
    for side, s in (("l", -1.0), ("r", 1.0)):
        shoulder, elbow, tip = (s * 0.05, 0.03, 0.025), (s * 0.31, 0.0, 0.025), (s * 0.65, -0.08, 0.025)
        # Flat wings: capsules along x squashed across (their local x is the world's z).
        sc.capsule(bm, shoulder, elbow, 0.08, scale=(0.11, 1.0))
        sc.capsule(bm, elbow, tip, 0.06, scale=(0.13, 1.0))
        bones += [
            (f"wing-{side}", shoulder, elbow, "body"),
            (f"hand-{side}", elbow, tip, f"wing-{side}"),
        ]
    mesh = sc.body("bird-body", bm, 0.005, 6000)
    sc.unwrap(mesh)
    sc.grain(mesh, bones)
    eyes = [(sc.on_surface(mesh, (x, 0.19, 0.06), (-x * 10.0, 0.0, 0.0)), 0.009) for x in (-0.1, 0.1)]
    bill = ((0.0, 0.25, 0.035), 0.05)
    spot = ((0.0, 0.27, 0.022), 0.008)
    sc.paint(mesh, "bird feathers", feathers(eyes, bill, spot), folder)
    rig = sc.armature("bird", bones)
    sc.skin(mesh, rig)
    x, y, z = (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0)

    def wings(raise_, pitch, fold, twist=0.0):
        # A left wing (at -x) rises by a turn about +y, a right one by a turn about -y; a wing
        # pitches its leading edge down by a turn about its span (-x on the left, +x on the
        # right, both nose down); a hand folds back by a turn about z.
        pose = {}
        for side, s in (("l", -1.0), ("r", 1.0)):
            arm = sc.turn(rig, f"wing-{side}", y, -s * raise_)
            pitch_turn = sc.turn(rig, f"wing-{side}", x, -pitch)
            pose[f"wing-{side}"] = arm @ pitch_turn
            pose[f"hand-{side}"] = (
                sc.turn(rig, f"hand-{side}", z, s * fold)
                @ sc.turn(rig, f"hand-{side}", y, -s * 0.5 * raise_)
                @ sc.turn(rig, f"hand-{side}", x, -twist)
            )
        return pose

    def flap(t):
        a = 2.0 * math.pi * t
        # Up at the start, down at the half; the hands fold on the way up. The wings twist
        # with the beat, the hands most (they move fastest): leading edge down on the way
        # down and up on the way up, so the air meets them from ahead and they pull.
        down = math.sin(a)
        pose = wings(0.75 * math.cos(a), 0.1 * down, 0.35 * max(0.0, -down), 0.32 * down)
        pose["tail"] = sc.turn(rig, "tail", x, 0.05 * math.sin(a))
        pose["head"] = sc.turn(rig, "head", x, -0.04 * math.cos(a))
        return pose

    def glide(t):
        a = 2.0 * math.pi * t
        pose = wings(0.1 + 0.04 * math.sin(2.0 * a), 0.0, 0.05 + 0.04 * math.sin(a))
        pose["head"] = sc.turn(rig, "head", z, 0.3 * math.sin(a))
        pose["tail"] = sc.turn(rig, "tail", y, 0.06 * math.sin(a))
        return pose

    sc.clip(rig, "bird-flap", 1.0 / 3.0, flap)
    sc.clip(rig, "bird-glide", 2.0, glide)
    return mesh, rig


def main(out):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.scene.render.fps = sc.FPS
    folder = tempfile.mkdtemp(prefix="bird-")
    objects = [*bird(folder)]
    for o in objects:
        if o.type == "MESH":
            print(f"{o.name}: {len(o.data.vertices)} vertices, {len(o.data.polygons)} faces")
    bpy.ops.object.select_all(action="DESELECT")
    for o in objects:
        o.select_set(True)
    for m in bpy.data.materials:
        m.use_backface_culling = True
    bpy.ops.export_scene.gltf(
        filepath=out,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=False,
        export_normals=True,
        export_materials="EXPORT",
        export_skins=True,
        export_animations=True,
        export_animation_mode="ACTIONS",
        export_force_sampling=True,
        export_frame_step=1,
        export_def_bones=False,
    )


if __name__ == "__main__":
    args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    main(args[0] if args else "bird.glb")
