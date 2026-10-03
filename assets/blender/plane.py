# The physics lab's aeroplane (Phase 3's step 5), modelled in Blender from code and exported as
# glTF:
#
#   blender --background --factory-startup --python assets/blender/plane.py -- OUT.glb
#
# A small high-wing monoplane, 7.3 m long with a 10 m span: a lofted fuselage, a straight
# wing on struts, a tailplane and a fin, fixed landing gear with spats. Three meshes go out:
#   plane        the aircraft without its propeller, its origin at the wing's quarter chord
#                on the fuselage's axis;
#   plane-prop   the propeller (two blades and a spinner), its axis along the flight, centred on
#                its origin (it spins in the engine);
#   plane-shell  a coarse closed hull round the fuselage and the gear, for collision.
# Blender's +Y (the nose) becomes glTF's -Z, Forge's forward; +Z up becomes +Y.

import math
import sys

import bmesh
import bpy

LENGTH = 7.3
NOSE_Y = 2.0  # the nose, ahead of the origin (the wing's quarter chord)
SPAN = 10.0
CHORD = 1.5
WING_Z = 1.05  # the wing's height over the fuselage's axis
PROP_Y = NOSE_Y + 0.12


def material(name, color, roughness, metallic=0.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    return m


def fuselage(mat):
    """A loft of rounded sections from the nose to the tail."""
    mesh = bpy.data.meshes.new("fuselage")
    bm = bmesh.new()
    stations = 28
    around = 24
    rings = []
    for i in range(stations + 1):
        t = i / stations  # 0 nose, 1 tail
        y = NOSE_Y - LENGTH * t
        # Width and height: a blunt nose, the cabin's bulk, a long taper to the tail.
        if t < 0.18:
            s = 0.55 + 0.45 * math.sin(t / 0.18 * math.pi / 2)
        else:
            s = 1.0 - 0.82 * ((t - 0.18) / 0.82) ** 1.3
        half_w = 0.58 * s
        top = 0.62 * s + (0.25 if 0.1 < t < 0.45 else 0.0) * math.sin(min(1.0, (t - 0.1) / 0.1) * math.pi / 2) * s
        bottom = -0.55 * s
        # The tail sweeps up.
        lift = 0.45 * max(0.0, (t - 0.55) / 0.45) ** 1.5
        ring = []
        for k in range(around):
            a = 2 * math.pi * k / around
            c, sn = math.cos(a), math.sin(a)
            z = (top if sn > 0 else -bottom) * sn
            ring.append(bm.verts.new((half_w * c, y, z + lift)))
        rings.append(ring)
    for a, b in zip(rings, rings[1:]):
        for k in range(around):
            bm.faces.new((a[k], a[(k + 1) % around], b[(k + 1) % around], b[k]))
    bm.faces.new(list(reversed(rings[0])))
    bm.faces.new(rings[-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new("fuselage", mesh)
    bpy.context.collection.objects.link(obj)
    obj.data.materials.append(mat)
    return obj


def box(name, size, at, mat, bevel=0.02, rotation=(0.0, 0.0, 0.0)):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=at, rotation=rotation)
    obj = bpy.context.active_object
    obj.name = name
    obj.scale = size
    bpy.ops.object.transform_apply(scale=True, rotation=True)
    obj.data.materials.append(mat)
    if bevel > 0.0:
        mod = obj.modifiers.new("bevel", "BEVEL")
        mod.width = bevel
        mod.segments = 3
        bpy.ops.object.modifier_apply(modifier=mod.name)
    return obj


def surface(name, span, chord, thickness, at, mat, taper=1.0, vertical=False):
    """A wing-like slab: rounded leading edge, its chord along y, tapered to the tip."""
    obj = box(name, (span, chord, thickness), at, mat, bevel=thickness * 0.45)
    for v in obj.data.vertices:
        f = abs(v.co.x - at[0]) / (span / 2) if not vertical else abs(v.co.z - at[2]) / (span / 2)
        scale = 1.0 - (1.0 - taper) * f
        v.co.y = at[1] + (v.co.y - at[1]) * scale
    if vertical:
        obj.rotation_euler = (0.0, math.pi / 2, 0.0)
    return obj


def cylinder(name, radius, depth, at, mat, rotation, vertices=24):
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=vertices, radius=radius, depth=depth, location=at, rotation=rotation
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
    white = material("plane-white", (0.85, 0.86, 0.87), 0.3)
    stripe = material("plane-stripe", (0.85, 0.42, 0.05), 0.35)
    glass = material("plane-glass", (0.04, 0.06, 0.08), 0.05)
    dark = material("plane-dark", (0.05, 0.05, 0.06), 0.6)
    metal = material("plane-metal", (0.6, 0.6, 0.62), 0.3, 0.8)

    parts = [fuselage(white)]
    # The windscreen and the side windows: a dark band round the cabin.
    parts.append(box("windows", (1.12, 1.25, 0.42), (0.0, 0.45, 0.72), glass, bevel=0.12))
    # The wing on top, the struts under it.
    parts.append(surface("wing", SPAN, CHORD, 0.16, (0.0, -0.1, WING_Z), white, taper=0.75))
    parts.append(box("wing-stripe", (SPAN * 0.98, 0.25, 0.165), (0.0, -0.72, WING_Z), stripe, bevel=0.06))
    for side in (-1.0, 1.0):
        strut = box(
            f"strut{side}", (0.06, 0.12, 1.9), (side * 1.55, -0.05, 0.25), metal, bevel=0.02,
            rotation=(0.0, side * math.radians(54.0), 0.0),
        )
        parts.append(strut)
    # The tail: a tailplane and a fin with its stripe.
    tail_y = NOSE_Y - LENGTH + 0.55
    parts.append(surface("tailplane", 3.4, 0.85, 0.08, (0.0, tail_y, 0.62), white, taper=0.7))
    # The fin: upright from the tail's top, its chord shrinking and its leading edge swept back
    # towards the tip.
    fin_bottom, fin_height = 0.75, 1.3
    fin = box("fin", (0.08, 1.2, fin_height), (0.0, tail_y - 0.1, fin_bottom + fin_height / 2), white, bevel=0.03)
    for v in fin.data.vertices:
        f = (v.co.z - fin_bottom) / fin_height
        back = tail_y - 0.7
        v.co.y = back + (v.co.y - back) * (1.0 - 0.45 * f)
    parts.append(fin)
    parts.append(box("fin-stripe", (0.085, 0.75, 0.16), (0.0, tail_y - 0.45, fin_bottom + 0.95), stripe, bevel=0.03))
    # The gear: two main legs with spats under the cabin, a nose leg.
    for side in (-1.0, 1.0):
        parts.append(box(f"leg{side}", (0.06, 0.12, 0.8), (side * 0.75, -0.2, -0.85), metal, rotation=(0.0, side * math.radians(-28.0), 0.0)))
        parts.append(box(f"spat{side}", (0.2, 0.62, 0.34), (side * 1.0, -0.2, -1.2), white, bevel=0.1))
    parts.append(box("nose-leg", (0.06, 0.08, 0.65), (0.0, NOSE_Y - 0.35, -0.85), metal))
    parts.append(box("nose-spat", (0.16, 0.5, 0.3), (0.0, NOSE_Y - 0.35, -1.2), white, bevel=0.08))
    plane = join(parts, "plane")
    bpy.ops.object.shade_auto_smooth(angle=math.radians(35))

    # The propeller: two blades twisted a little, and a spinner, centred on its origin.
    blades = []
    for side in (-1.0, 1.0):
        b = box(f"blade{side}", (0.13, 0.04, 0.9), (0.0, 0.0, side * 0.48), dark, bevel=0.015, rotation=(0.0, side * math.radians(12.0), 0.0))
        blades.append(b)
    spinner = cylinder("spinner", 0.16, 0.36, (0.0, 0.0, 0.0), white, rotation=(math.pi / 2, 0.0, 0.0))
    prop = join([spinner] + blades, "plane-prop")
    bpy.ops.object.shade_auto_smooth(angle=math.radians(35))

    # The collision hull: a box round the fuselage's front and the gear, and the tail, as one
    # closed mesh (its points' hull is taken).
    shell = box("plane-shell", (1.2, 4.2, 2.2), (0.0, 0.0, -0.25), dark, bevel=0.0)
    tail = box("shell-tail", (0.6, 2.6, 0.9), (0.0, NOSE_Y - LENGTH + 1.3, 0.55), dark, bevel=0.0)
    shell = join([shell, tail], "plane-shell")

    bpy.ops.object.select_all(action="DESELECT")
    for o in (plane, prop, shell):
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
    main(args[0] if args else "plane.glb")
