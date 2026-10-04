# The physics lab's car (Phase 3's step 5), modelled in Blender from code and exported as glTF:
#
#   blender --background --factory-startup --python assets/blender/car.py -- OUT.glb
#
# A small two-door hatchback, 3.9 m long and 1.75 m wide, built from bevelled boxes and
# cylinders: the body (paint), the glasshouse (dark glass), the bumpers, lights and grille,
# and one wheel. Three meshes go out:
#   car        the body, its origin on the ground under the middle of its wheelbase, the wheel
#              arches left open;
#   car-wheel  one wheel (tyre and rim), its axle along x, centred on its origin;
#   car-shell  a coarse convex box of the body for its collision (chassis).
# Blender's +Y (the front) becomes glTF's -Z, Forge's forward; +Z up becomes +Y.

import math
import sys

import bmesh
import bpy

LENGTH = 3.9
WIDTH = 1.75
WHEELBASE = 2.45
TRACK = 1.48
WHEEL_RADIUS = 0.31
WHEEL_WIDTH = 0.2
RIDE = 0.18  # the body's floor over the ground


def material(name, color, roughness, metallic=0.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    return m


def box(name, size, at, mat, bevel=0.04, segments=3):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=at)
    obj = bpy.context.active_object
    obj.name = name
    obj.scale = size
    bpy.ops.object.transform_apply(scale=True)
    obj.data.materials.append(mat)
    if bevel > 0.0:
        mod = obj.modifiers.new("bevel", "BEVEL")
        mod.width = bevel
        mod.segments = segments
        bpy.ops.object.modifier_apply(modifier=mod.name)
    return obj


def taper(obj, top_scale_x, top_scale_y, top_shift_y):
    """Narrows a box's top face towards the roof (a glasshouse's tumblehome and rake)."""
    zs = [v.co.z for v in obj.data.vertices]
    low, high = min(zs), max(zs)
    for v in obj.data.vertices:
        t = (v.co.z - low) / (high - low)
        v.co.x *= 1.0 - (1.0 - top_scale_x) * t
        v.co.y = v.co.y * (1.0 - (1.0 - top_scale_y) * t) + top_shift_y * t


def cylinder(name, radius, depth, at, mat, vertices=48):
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=vertices, radius=radius, depth=depth, location=at, rotation=(0.0, math.pi / 2, 0.0)
    )
    obj = bpy.context.active_object
    obj.name = name
    bpy.ops.object.transform_apply(rotation=True)
    obj.data.materials.append(mat)
    return obj


def join(objects, name):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objects:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    bpy.ops.object.join()
    obj = bpy.context.active_object
    obj.name = name
    return obj


def main(out):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    paint = material("car-paint", (0.55, 0.07, 0.05), 0.25, 0.3)
    glass = material("car-glass", (0.03, 0.04, 0.05), 0.05)
    trim = material("car-trim", (0.04, 0.04, 0.045), 0.5)
    light = material("car-light", (0.9, 0.88, 0.8), 0.1)
    tail = material("car-tail", (0.6, 0.03, 0.02), 0.2)
    tyre = material("car-tyre", (0.02, 0.02, 0.02), 0.9)
    rim = material("car-rim", (0.65, 0.66, 0.68), 0.25, 0.9)

    half = WHEELBASE / 2.0
    # The lower body, its sills, and the bonnet's slope.
    lower = box("lower", (WIDTH, LENGTH, 0.55), (0.0, 0.0, RIDE + 0.3), paint, bevel=0.09)
    # The glasshouse over the rear two thirds, raked at both ends, its sides drawn in.
    cabin = box("cabin", (WIDTH * 0.9, LENGTH * 0.55, 0.5), (0.0, -0.25, RIDE + 0.8), glass, bevel=0.06)
    taper(cabin, 0.86, 0.7, -0.1)
    roof = box("roof", (WIDTH * 0.78, LENGTH * 0.36, 0.06), (0.0, -0.33, RIDE + 1.06), paint, bevel=0.03)
    # Bumpers, grille, lights.
    front = box("front-bumper", (WIDTH * 0.98, 0.18, 0.2), (0.0, LENGTH / 2.0 - 0.02, RIDE + 0.15), trim)
    rear = box("rear-bumper", (WIDTH * 0.98, 0.18, 0.2), (0.0, -LENGTH / 2.0 + 0.02, RIDE + 0.15), trim)
    grille = box("grille", (WIDTH * 0.45, 0.04, 0.14), (0.0, LENGTH / 2.0 + 0.0, RIDE + 0.4), trim, bevel=0.01)
    lamps = []
    for side in (-1.0, 1.0):
        lamps.append(box(f"head{side}", (0.32, 0.05, 0.12), (side * 0.6, LENGTH / 2.0 - 0.02, RIDE + 0.45), light, bevel=0.015))
        lamps.append(box(f"tail{side}", (0.26, 0.05, 0.16), (side * 0.65, -LENGTH / 2.0 + 0.02, RIDE + 0.5), tail, bevel=0.015))
    # The wheel arches, cut out of the lower body alone (a boolean on the joined parts, which
    # overlap, drops some of them), then everything joined.
    for y in (-half, half):
        for side in (-1.0, 1.0):
            cutter = cylinder("arch", WHEEL_RADIUS + 0.06, 0.6, (side * TRACK / 2.0, y, WHEEL_RADIUS), trim)
            mod = lower.modifiers.new("arch", "BOOLEAN")
            mod.operation = "DIFFERENCE"
            mod.object = cutter
            bpy.context.view_layer.objects.active = lower
            bpy.ops.object.modifier_apply(modifier=mod.name)
            bpy.data.objects.remove(cutter, do_unlink=True)
    body = join([lower, cabin, roof, front, rear, grille] + lamps, "car")
    bpy.ops.object.shade_auto_smooth(angle=math.radians(30))

    # One wheel at the origin: a tyre with rounded shoulders, a rim recessed into it.
    tyre_obj = cylinder("tyre", WHEEL_RADIUS, WHEEL_WIDTH, (0.0, 0.0, 0.0), tyre, vertices=40)
    mod = tyre_obj.modifiers.new("bevel", "BEVEL")
    mod.width = 0.05
    mod.segments = 4
    bpy.context.view_layer.objects.active = tyre_obj
    bpy.ops.object.modifier_apply(modifier=mod.name)
    rim_obj = cylinder("rim", WHEEL_RADIUS * 0.62, WHEEL_WIDTH + 0.01, (0.0, 0.0, 0.0), rim, vertices=10)
    wheel = join([tyre_obj, rim_obj], "car-wheel")
    bpy.ops.object.shade_auto_smooth(angle=math.radians(40))
    wheel.location = (0.0, 0.0, 0.0)

    # The chassis' collision: a box round the lower body and a smaller one round the cabin, as
    # one closed mesh of their corners (a convex hull is taken from its points).
    shell = box("car-shell", (WIDTH * 0.96, LENGTH * 0.96, 0.95), (0.0, 0.0, RIDE + 0.5), trim, bevel=0.0)

    bpy.ops.object.select_all(action="DESELECT")
    for o in (body, wheel, shell):
        o.select_set(True)
    # Closed meshes: one-sided in the glTF (Forge draws a double-sided material's back faces, and
    # gives it no inside for the light probes, #171); Blender's default exports them double-sided.
    for m in bpy.data.materials:
        m.use_backface_culling = True
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
    main(args[0] if args else "car.glb")
