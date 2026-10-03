# The physics lab's creatures (Phase 3's step 7), modelled in Blender from code and exported as
# glTF:
#
#   blender --background --factory-startup --python assets/blender/creatures.py -- OUT.glb
#
# Two puppets of rigid parts, each part a mesh of its own (a body of a ragdoll in the lab), and
# for each part but the root an empty at the joint that holds it to its parent:
#   mannequin-*   a 1.8 m artist's mannequin of pale wood: pelvis, chest, head, upper and lower
#                 arms, thighs and shins (its feet on them), eleven parts;
#   dog-*         a 1 m dog: torso, head, tail, upper and lower legs, eleven parts;
#   *-joint-PART  the joint of PART with its parent.
# Each creature stands on the ground at its origin, facing Blender's +Y (Forge's forward, -Z).
# Blender's +Z up becomes Forge's +Y.

import math
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector


def material(name, color, roughness):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    return m


def mesh_object(name, bm, mat):
    data = bpy.data.meshes.new(name)
    bm.to_mesh(data)
    bm.free()
    obj = bpy.data.objects.new(name, data)
    bpy.context.scene.collection.objects.link(obj)
    data.materials.append(mat)
    for p in data.polygons:
        p.use_smooth = True
    return obj


def capsule(name, a, b, radius, mat, scale=(1.0, 1.0), segments=16, rings=10):
    """A capsule from point a to point b (Blender coordinates), its section scaled across."""
    a, b = Vector(a), Vector(b)
    length = (b - a).length
    bm = bmesh.new()
    bmesh.ops.create_uvsphere(bm, u_segments=segments, v_segments=rings, radius=radius)
    for v in bm.verts:
        v.co.x *= scale[0]
        v.co.y *= scale[1]
        v.co.z += length * 0.5 if v.co.z > 1e-6 else (-length * 0.5 if v.co.z < -1e-6 else 0.0)
    axis = (b - a).normalized()
    rot = Vector((0.0, 0.0, 1.0)).rotation_difference(axis).to_matrix().to_4x4()
    bmesh.ops.transform(bm, matrix=Matrix.Translation((a + b) * 0.5) @ rot, verts=bm.verts)
    return mesh_object(name, bm, mat)


def blob(name, at, size, mat, segments=20, rings=12):
    """An ellipsoid of half sizes `size` at `at`."""
    bm = bmesh.new()
    bmesh.ops.create_uvsphere(bm, u_segments=segments, v_segments=rings, radius=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(at), verts=bm.verts)
    return mesh_object(name, bm, mat)


def join(objects, name):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objects:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    bpy.ops.object.join()
    obj = bpy.context.active_object
    obj.name = name
    return obj


def joint(name, at):
    empty = bpy.data.objects.new(name, None)
    empty.location = at
    bpy.context.scene.collection.objects.link(empty)
    return empty


def mannequin(wood, dark):
    parts, joints = [], []
    p = lambda part: f"mannequin-{part}"
    parts.append(blob(p("pelvis"), (0.0, 0.0, 0.98), (0.16, 0.1, 0.1), wood))
    parts.append(join([
        capsule("chest-a", (0.0, 0.0, 1.12), (0.0, 0.0, 1.42), 0.13, wood, scale=(1.45, 0.85)),
        blob("neck", (0.0, 0.0, 1.52), (0.045, 0.045, 0.07), dark),
    ], p("chest")))
    joints.append(joint(p("joint-chest"), (0.0, 0.0, 1.07)))
    parts.append(blob(p("head"), (0.0, 0.01, 1.67), (0.095, 0.11, 0.125), wood))
    joints.append(joint(p("joint-head"), (0.0, 0.0, 1.55)))
    for side, x in (("l", -1.0), ("r", 1.0)):
        shoulder, elbow, wrist = (0.23 * x, 0.0, 1.43), (0.25 * x, 0.0, 1.15), (0.26 * x, 0.02, 0.89)
        parts.append(join([
            capsule("upper", shoulder, elbow, 0.05, wood),
            blob("shoulder", shoulder, (0.06, 0.06, 0.06), dark),
        ], p(f"upper-arm-{side}")))
        joints.append(joint(p(f"joint-upper-arm-{side}"), shoulder))
        parts.append(join([
            capsule("lower", elbow, wrist, 0.042, wood),
            blob("elbow", elbow, (0.048, 0.048, 0.048), dark),
            blob("hand", (wrist[0], wrist[1], wrist[2] - 0.06), (0.03, 0.05, 0.08), wood),
        ], p(f"lower-arm-{side}")))
        joints.append(joint(p(f"joint-lower-arm-{side}"), elbow))
        hip, knee, ankle = (0.1 * x, 0.0, 0.93), (0.1 * x, 0.0, 0.5), (0.1 * x, 0.0, 0.1)
        parts.append(join([
            capsule("thigh", hip, knee, 0.072, wood),
            blob("hip", hip, (0.07, 0.07, 0.07), dark),
        ], p(f"thigh-{side}")))
        joints.append(joint(p(f"joint-thigh-{side}"), hip))
        parts.append(join([
            capsule("shin", knee, ankle, 0.055, wood),
            blob("knee", knee, (0.06, 0.06, 0.06), dark),
            blob("foot", (0.1 * x, 0.07, 0.04), (0.05, 0.12, 0.04), wood),
        ], p(f"shin-{side}")))
        joints.append(joint(p(f"joint-shin-{side}"), knee))
    return parts, joints


def dog(fur, dark):
    parts, joints = [], []
    p = lambda part: f"dog-{part}"
    # Blender's +y is the front: the head at +y, the tail at -y.
    parts.append(capsule(p("torso"), (0.0, -0.28, 0.56), (0.0, 0.28, 0.58), 0.15, fur, scale=(0.85, 1.0)))
    parts.append(join([
        blob("skull", (0.0, 0.48, 0.78), (0.09, 0.1, 0.09), fur),
        capsule("snout", (0.0, 0.54, 0.75), (0.0, 0.66, 0.72), 0.045, fur),
        blob("nose", (0.0, 0.69, 0.73), (0.022, 0.018, 0.018), dark),
        capsule("neck", (0.0, 0.36, 0.62), (0.0, 0.46, 0.74), 0.06, fur),
        blob("ear-l", (-0.07, 0.46, 0.87), (0.025, 0.035, 0.05), dark),
        blob("ear-r", (0.07, 0.46, 0.87), (0.025, 0.035, 0.05), dark),
    ], p("head")))
    joints.append(joint(p("joint-head"), (0.0, 0.36, 0.62)))
    parts.append(capsule(p("tail"), (0.0, -0.42, 0.62), (0.0, -0.62, 0.78), 0.028, fur))
    joints.append(joint(p("joint-tail"), (0.0, -0.42, 0.62)))
    for name, x, y in (("front-l", -0.09, 0.26), ("front-r", 0.09, 0.26), ("hind-l", -0.09, -0.26), ("hind-r", 0.09, -0.26)):
        top, knee, paw = (x, y, 0.5), (x, y, 0.27), (x, y + 0.02, 0.04)
        parts.append(capsule(p(f"upper-{name}"), top, knee, 0.045, fur))
        joints.append(joint(p(f"joint-upper-{name}"), top))
        parts.append(join([
            capsule("lower", knee, paw, 0.034, fur),
            blob("paw", (x, y + 0.03, 0.025), (0.035, 0.05, 0.025), dark),
        ], p(f"lower-{name}")))
        joints.append(joint(p(f"joint-lower-{name}"), knee))
    return parts, joints


def main(out):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    wood = material("pale wood", (0.62, 0.48, 0.32), 0.55)
    joint_wood = material("dark wood", (0.28, 0.18, 0.1), 0.5)
    fur = material("fur (tan)", (0.5, 0.3, 0.14), 0.9)
    fur_dark = material("fur (dark)", (0.1, 0.07, 0.05), 0.8)
    parts_a, joints_a = mannequin(wood, joint_wood)
    parts_b, joints_b = dog(fur, fur_dark)
    bpy.ops.object.select_all(action="DESELECT")
    for o in parts_a + joints_a + parts_b + joints_b:
        o.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=out,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=True,
        export_normals=True,
        export_materials="EXPORT",
    )


if __name__ == "__main__":
    args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    main(args[0] if args else "creatures.glb")
