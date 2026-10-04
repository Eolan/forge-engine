# The physics lab's creatures as skinned meshes (#165, Phase 3's step 7), modelled and
# animated in Blender from code and exported as glTF:
#
#   blender --background --factory-startup --python assets/blender/skinned_creatures.py -- OUT.glb
#
# The mannequin and the dog of `creatures.py`, each now one continuous body (its shapes joined
# and voxel-remeshed, then decimated) on an armature of eleven bones named as the rigid parts
# were (pelvis, chest, head, upper-arm-l...; torso, head, tail, upper-front-l...), weighted by
# Blender's bone heat so it bends at the joints. Each bone's head is its joint with its parent,
# where the rigid version had an empty. The mannequin stands in an A-pose (arms 30 degrees out:
# straight down they would melt into the torso) and keeps its dark joints as a second
# material; the dog its dark nose, ears and paws.
#
# Clips, sampled at 24 frames a second and looping (last key = first):
#   mannequin-walk  1 s: legs and arms swinging in opposition, knees and elbows bending, the
#                   chest turning against the hips, the pelvis bobbing;
#   mannequin-idle  4 s: breathing, the head looking about, the arms swaying;
#   dog-walk        0.75 s: a walk on diagonal pairs, the tail and the head swaying;
#   dog-idle        2 s: the tail wagging, the head tilting, breathing.
#
# Each creature stands on the ground at its origin, facing Blender's +Y (Forge's forward, -Z).
# Blender's +Z up becomes Forge's +Y.

import math
import sys

import bmesh
import bpy
from mathutils import Matrix, Quaternion, Vector

FPS = 24


def material(name, color, roughness):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    return m


def capsule(bm, a, b, radius, scale=(1.0, 1.0), segments=24, rings=16):
    """Adds a capsule from point a to point b to `bm`, its section scaled across."""
    a, b = Vector(a), Vector(b)
    length = (b - a).length
    part = bmesh.new()
    bmesh.ops.create_uvsphere(part, u_segments=segments, v_segments=rings, radius=radius)
    for v in part.verts:
        v.co.x *= scale[0]
        v.co.y *= scale[1]
        v.co.z += length * 0.5 if v.co.z > 1e-6 else (-length * 0.5 if v.co.z < -1e-6 else 0.0)
    axis = (b - a).normalized()
    rot = Vector((0.0, 0.0, 1.0)).rotation_difference(axis).to_matrix().to_4x4()
    bmesh.ops.transform(part, matrix=Matrix.Translation((a + b) * 0.5) @ rot, verts=part.verts)
    merge(bm, part)


def blob(bm, at, size, segments=24, rings=16):
    """Adds an ellipsoid of half sizes `size` at `at` to `bm`."""
    part = bmesh.new()
    bmesh.ops.create_uvsphere(part, u_segments=segments, v_segments=rings, radius=1.0)
    bmesh.ops.scale(part, vec=Vector(size), verts=part.verts)
    bmesh.ops.translate(part, vec=Vector(at), verts=part.verts)
    merge(bm, part)


def merge(bm, part):
    data = bpy.data.meshes.new("part")
    part.to_mesh(data)
    part.free()
    bm.from_mesh(data)
    bpy.data.meshes.remove(data)


def body(name, bm, voxel, triangles, materials, dark):
    """One continuous body from the overlapping shapes in `bm`: voxel-remeshed (which joins
    them), decimated to about `triangles` triangles, smooth, its faces in `dark(centre)` taking the
    second material."""
    data = bpy.data.meshes.new(name)
    bm.to_mesh(data)
    bm.free()
    obj = bpy.data.objects.new(name, data)
    bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.objects.active = obj
    remesh = obj.modifiers.new("remesh", "REMESH")
    remesh.mode = "VOXEL"
    remesh.voxel_size = voxel
    bpy.ops.object.modifier_apply(modifier=remesh.name)
    # The voxels' steps smoothed away first, or the decimation lays long triangles along them
    # that shade as streaks.
    smooth = obj.modifiers.new("smooth", "SMOOTH")
    smooth.factor = 0.5
    smooth.iterations = 4
    bpy.ops.object.modifier_apply(modifier=smooth.name)
    decimate = obj.modifiers.new("decimate", "DECIMATE")
    # The remesh makes quads; the ratio counts triangles.
    decimate.ratio = min(1.0, triangles / max(1, 2 * len(obj.data.polygons)))
    bpy.ops.object.modifier_apply(modifier=decimate.name)
    obj.data.validate()
    for m in materials:
        obj.data.materials.append(m)
    for p in obj.data.polygons:
        p.use_smooth = True
        p.material_index = 1 if dark(Vector(p.center)) else 0
    return obj


def near(points):
    """Whether a point lies within any of the spheres `points` ((centre, radius) pairs)."""
    return lambda c: any((c - Vector(p)).length < r for p, r in points)


def armature(name, bones):
    """An armature of `bones` (name, head, tail, parent name), its rest pose as given."""
    data = bpy.data.armatures.new(name)
    obj = bpy.data.objects.new(name, data)
    bpy.context.scene.collection.objects.link(obj)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.mode_set(mode="EDIT")
    for bone, head, tail, parent in bones:
        b = data.edit_bones.new(bone)
        b.head, b.tail = Vector(head), Vector(tail)
        if parent:
            b.parent = data.edit_bones[parent]
            b.use_connect = False
    bpy.ops.object.mode_set(mode="OBJECT")
    return obj


def skin(mesh, rig):
    """Parents `mesh` to `rig` with bone-heat weights; every vertex must get some."""
    bpy.ops.object.select_all(action="DESELECT")
    mesh.select_set(True)
    rig.select_set(True)
    bpy.context.view_layer.objects.active = rig
    bpy.ops.object.parent_set(type="ARMATURE_AUTO")
    # Bone heat leaves some weights a hair over 1; validating clamps them.
    mesh.data.validate()
    unweighted = sum(1 for v in mesh.data.vertices if not any(g.weight > 0 for g in v.groups))
    if unweighted:
        raise RuntimeError(f"{mesh.name}: {unweighted} vertices without weights")


def turn(rig, bone, axis, angle):
    """A turn of `angle` about `axis` (the armature's frame) as `bone`'s local rotation."""
    rest = rig.data.bones[bone].matrix_local.to_quaternion()
    return rest.inverted() @ Quaternion(Vector(axis), angle) @ rest


def clip(rig, name, seconds, pose):
    """Keys `pose(t)` (bone name to local rotation, and "lift" to the root's rise in metres)
    on `rig` every frame of `seconds`, t from 0 to 1, as the action `name`, kept in an NLA
    track for the exporter."""
    rig.animation_data_create()
    rig.animation_data.action = None
    frames = round(seconds * FPS)
    root = rig.pose.bones[0]
    for f in range(frames + 1):
        t = (f % frames) / frames
        targets = pose(t)
        for pb in rig.pose.bones:
            pb.rotation_mode = "QUATERNION"
            pb.rotation_quaternion = targets.get(pb.name, Quaternion())
            pb.keyframe_insert("rotation_quaternion", frame=f + 1)
        lift = targets.get("lift", 0.0)
        root.location = rig.data.bones[0].matrix_local.to_quaternion().inverted() @ Vector((0.0, 0.0, lift))
        root.keyframe_insert("location", frame=f + 1)
    action = rig.animation_data.action
    action.name = name
    action.use_fake_user = True
    track = rig.animation_data.nla_tracks.new()
    track.name = name
    track.strips.new(name, 1, action)
    track.mute = True
    rig.animation_data.action = None
    for pb in rig.pose.bones:
        pb.rotation_quaternion = Quaternion()
        pb.location = Vector()


def mannequin(wood, dark):
    bm = bmesh.new()
    blob(bm, (0.0, 0.0, 0.98), (0.16, 0.1, 0.11))
    capsule(bm, (0.0, 0.0, 1.12), (0.0, 0.0, 1.42), 0.13, scale=(1.45, 0.85))
    capsule(bm, (0.0, 0.0, 1.45), (0.0, 0.0, 1.6), 0.045)
    blob(bm, (0.0, 0.01, 1.67), (0.095, 0.11, 0.125))
    bones = [
        ("pelvis", (0.0, 0.0, 0.93), (0.0, 0.0, 1.07), None),
        ("chest", (0.0, 0.0, 1.07), (0.0, 0.0, 1.5), "pelvis"),
        ("head", (0.0, 0.0, 1.55), (0.0, 0.0, 1.8), "chest"),
    ]
    joints = [((0.0, 0.0, 1.53), 0.06)]
    out = math.radians(30.0)
    for side, x in (("l", -1.0), ("r", 1.0)):
        down = Vector((math.sin(out) * x, 0.0, -math.cos(out)))
        shoulder = Vector((0.2 * x, 0.0, 1.43))
        elbow = shoulder + 0.28 * down
        wrist = elbow + 0.27 * down
        blob(bm, shoulder, (0.065, 0.065, 0.065))
        capsule(bm, shoulder, elbow, 0.05)
        blob(bm, elbow, (0.048, 0.048, 0.048))
        capsule(bm, elbow, wrist, 0.042)
        blob(bm, wrist + 0.07 * down, (0.03, 0.05, 0.08))
        hip, knee, ankle = (0.1 * x, 0.0, 0.93), (0.11 * x, 0.0, 0.5), (0.12 * x, 0.0, 0.1)
        blob(bm, hip, (0.075, 0.075, 0.075))
        capsule(bm, hip, knee, 0.072)
        blob(bm, knee, (0.06, 0.06, 0.06))
        capsule(bm, knee, ankle, 0.055)
        blob(bm, (0.12 * x, 0.07, 0.04), (0.05, 0.12, 0.04))
        bones += [
            (f"upper-arm-{side}", shoulder, elbow, "chest"),
            (f"lower-arm-{side}", elbow, wrist + 0.14 * down, f"upper-arm-{side}"),
            (f"thigh-{side}", hip, knee, "pelvis"),
            (f"shin-{side}", knee, ankle, f"thigh-{side}"),
        ]
        joints += [(shoulder, 0.075), (elbow, 0.055), (knee, 0.064)]
    mesh = body("mannequin-body", bm, 0.011, 10000, [wood, dark], near(joints))
    rig = armature("mannequin", bones)
    skin(mesh, rig)
    x, z = (1.0, 0.0, 0.0), (0.0, 0.0, 1.0)

    def walk(t):
        s = math.sin(2.0 * math.pi * t)
        c = math.cos(2.0 * math.pi * t)
        # A knee bends (the shin back, a negative turn about +x: a positive one swings a
        # hanging limb forward) while its leg swings forward.
        bend = lambda phase: 0.15 + 0.55 * max(0.0, math.sin(2.0 * math.pi * t + phase))
        return {
            "thigh-l": turn(rig, "thigh-l", x, -0.45 * s),
            "thigh-r": turn(rig, "thigh-r", x, 0.45 * s),
            "shin-l": turn(rig, "shin-l", x, -bend(math.pi)),
            "shin-r": turn(rig, "shin-r", x, -bend(0.0)),
            "upper-arm-l": turn(rig, "upper-arm-l", x, 0.35 * s),
            "upper-arm-r": turn(rig, "upper-arm-r", x, -0.35 * s),
            "lower-arm-l": turn(rig, "lower-arm-l", x, 0.3 + 0.15 * max(0.0, s)),
            "lower-arm-r": turn(rig, "lower-arm-r", x, 0.3 + 0.15 * max(0.0, -s)),
            "chest": turn(rig, "chest", z, 0.1 * s),
            "head": turn(rig, "head", z, -0.08 * s),
            "lift": 0.02 * c * c,
        }

    def idle(t):
        breath = math.sin(4.0 * math.pi * t)
        look = math.sin(2.0 * math.pi * t)
        return {
            "chest": turn(rig, "chest", x, 0.025 * breath),
            "head": turn(rig, "head", z, 0.45 * look) @ turn(rig, "head", x, 0.08 * breath),
            "upper-arm-l": turn(rig, "upper-arm-l", x, 0.05 * breath),
            "upper-arm-r": turn(rig, "upper-arm-r", x, -0.05 * breath),
            "lower-arm-l": turn(rig, "lower-arm-l", x, 0.15),
            "lower-arm-r": turn(rig, "lower-arm-r", x, 0.15),
        }

    clip(rig, "mannequin-walk", 1.0, walk)
    clip(rig, "mannequin-idle", 4.0, idle)
    return mesh, rig


def dog(fur, dark):
    bm = bmesh.new()
    # Blender's +y is the front: the head at +y, the tail at -y.
    capsule(bm, (0.0, -0.28, 0.56), (0.0, 0.28, 0.58), 0.15, scale=(0.85, 1.0))
    capsule(bm, (0.0, 0.3, 0.6), (0.0, 0.46, 0.74), 0.065)
    blob(bm, (0.0, 0.48, 0.78), (0.09, 0.1, 0.09))
    capsule(bm, (0.0, 0.54, 0.75), (0.0, 0.66, 0.72), 0.045)
    blob(bm, (0.0, 0.69, 0.73), (0.024, 0.02, 0.02))
    blob(bm, (-0.07, 0.46, 0.87), (0.028, 0.035, 0.05))
    blob(bm, (0.07, 0.46, 0.87), (0.028, 0.035, 0.05))
    capsule(bm, (0.0, -0.4, 0.61), (0.0, -0.62, 0.78), 0.03)
    bones = [
        ("torso", (0.0, -0.3, 0.57), (0.0, 0.3, 0.57), None),
        ("head", (0.0, 0.36, 0.62), (0.0, 0.66, 0.74), "torso"),
        ("tail", (0.0, -0.42, 0.62), (0.0, -0.64, 0.8), "torso"),
    ]
    for name, x, y in (
        ("front-l", -0.09, 0.26),
        ("front-r", 0.09, 0.26),
        ("hind-l", -0.09, -0.26),
        ("hind-r", 0.09, -0.26),
    ):
        top, knee, paw = (x, y, 0.5), (x, y, 0.27), (x, y + 0.02, 0.04)
        capsule(bm, (x, y, 0.56), knee, 0.05)
        capsule(bm, knee, paw, 0.036)
        blob(bm, (x, y + 0.03, 0.025), (0.037, 0.052, 0.027))
        bones += [
            (f"upper-{name}", top, knee, "torso"),
            (f"lower-{name}", knee, paw, f"upper-{name}"),
        ]
    dark_parts = near([((0.0, 0.69, 0.73), 0.032), ((-0.07, 0.46, 0.89), 0.045), ((0.07, 0.46, 0.89), 0.045)])
    mesh = body("dog-body", bm, 0.008, 10000, [fur, dark], lambda c: dark_parts(c) or c.z < 0.05)
    rig = armature("dog", bones)
    skin(mesh, rig)
    x, z = (1.0, 0.0, 0.0), (0.0, 0.0, 1.0)

    def walk(t):
        # Diagonal pairs a half cycle apart, each leg's lower part folding as it swings
        # forward.
        def leg(phase):
            a = 2.0 * math.pi * t + phase
            return 0.38 * math.sin(a), -0.55 * max(0.0, math.cos(a))

        pose = {}
        for name, phase in (("front-l", 0.0), ("hind-r", 0.0), ("front-r", math.pi), ("hind-l", math.pi)):
            swing, fold = leg(phase)
            # Hind knees bend the other way: their lower legs fold forward.
            if name.startswith("hind"):
                fold = -fold
            pose[f"upper-{name}"] = turn(rig, f"upper-{name}", x, -swing)
            pose[f"lower-{name}"] = turn(rig, f"lower-{name}", x, fold)
        s = math.sin(4.0 * math.pi * t)
        pose["tail"] = turn(rig, "tail", z, 0.3 * math.sin(2.0 * math.pi * t))
        pose["head"] = turn(rig, "head", x, 0.06 * s)
        pose["lift"] = 0.012 * s
        return pose

    def idle(t):
        return {
            "tail": turn(rig, "tail", z, 0.55 * math.sin(8.0 * math.pi * t)),
            "head": turn(rig, "head", (0.0, 1.0, 0.0), 0.25 * math.sin(2.0 * math.pi * t)),
            "torso": turn(rig, "torso", x, 0.015 * math.sin(4.0 * math.pi * t)),
        }

    clip(rig, "dog-walk", 0.75, walk)
    clip(rig, "dog-idle", 2.0, idle)
    return mesh, rig


def main(out):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.scene.render.fps = FPS
    wood = material("pale wood", (0.62, 0.48, 0.32), 0.55)
    joint_wood = material("dark wood", (0.28, 0.18, 0.1), 0.5)
    fur = material("fur (tan)", (0.5, 0.3, 0.14), 0.9)
    fur_dark = material("fur (dark)", (0.1, 0.07, 0.05), 0.8)
    objects = [*mannequin(wood, joint_wood), *dog(fur, fur_dark)]
    for o in objects:
        if o.type == "MESH":
            print(f"{o.name}: {len(o.data.vertices)} vertices, {len(o.data.polygons)} faces")
    bpy.ops.object.select_all(action="DESELECT")
    for o in objects:
        o.select_set(True)
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
    main(args[0] if args else "skinned-creatures.glb")
