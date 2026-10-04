# The physics lab's boat (Phase 3's step 3), modelled in Blender from code and exported as glTF:
#
#   blender --background --factory-startup --python assets/blender/boat.py -- OUT.glb
#
# A 5 m open motorboat: a round-bilged hull with a raised bow and a flat transom, a gunwale
# rim, three thwarts and an outboard motor. Two meshes go out:
#   boat           what is drawn, its parts by material (hull, wood, motor);
#   boat-buoyancy  a closed, coarse shell of the hull up to the gunwale: what floats it.
# Blender's +Y (the bow) becomes glTF's -Z, Forge's forward; +Z up becomes +Y.

import math
import sys

import bmesh
import bpy

LENGTH = 5.0  # metres, transom to stem
BEAM = 1.9  # metres at the widest
DEPTH = 0.75  # gunwale to keel amidships, metres
SHEER = 0.25  # how much the gunwale rises at the bow, metres
STATIONS = 48  # rings along the length
AROUND = 24  # points from gunwale to keel on each side


def half_beam(s):
    """Half the width at s (-1 the transom, 1 the stem)."""
    bow = max(s, 0.0)
    stern = max(-s, 0.0)
    return 0.5 * BEAM * (1.0 - bow**2.2) ** 0.7 * (1.0 - 0.18 * stern**2)


def gunwale(s):
    return SHEER * max(s, 0.0) ** 2


def depth(s):
    bow = max(s, 0.0)
    stern = max(-s, 0.0)
    return DEPTH * (1.0 - 0.45 * bow**3) * (1.0 - 0.25 * stern**2)


def section(s, around):
    """The ring at s from the port gunwale down to the keel and up to the starboard gunwale."""
    y = 0.5 * LENGTH * s
    hb, top, d = half_beam(s), gunwale(s), depth(s)
    points = []
    for k in range(2 * around + 1):
        u = k / around - 1.0  # -1 port gunwale, 0 keel, 1 starboard gunwale
        a = abs(u)
        # A round bilge: steep sides, a flatter bottom, a slight vee at the keel.
        x = math.copysign(hb * a**0.55, u)
        z = top - d * (1.0 - a**2.4)
        points.append((x, y, z))
    return points


def hull_mesh(name, stations, around, solid):
    """The hull as rings joined into a skin; the transom closed. `solid` gives it a thickness
    with the gunwale capped (what is drawn); otherwise the open top is closed by a deck at the
    gunwale (the buoyancy shell)."""
    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    rings = []
    for i in range(stations + 1):
        s = -1.0 + 2.0 * i / stations
        if i == stations:
            s = 0.999  # the stem: nearly a point
        rings.append([bm.verts.new(p) for p in section(s, around)])
    for a, b in zip(rings, rings[1:]):
        for k in range(len(a) - 1):
            bm.faces.new((a[k], a[k + 1], b[k + 1], b[k]))
    # The transom.
    bm.faces.new(list(reversed(rings[0])))
    if not solid:
        # The deck: the gunwale line closed over the top, so the shell holds a volume.
        top = [r[0] for r in rings] + [r[-1] for r in reversed(rings)]
        bm.faces.new(top)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    if solid:
        mod = obj.modifiers.new("thickness", "SOLIDIFY")
        mod.thickness = 0.04
        mod.offset = -1.0
        mod.use_rim = True
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.modifier_apply(modifier=mod.name)
    return obj


def material(name, color, roughness, metallic=0.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    return m


def box(name, size, at, mat):
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=at)
    obj = bpy.context.active_object
    obj.name = name
    obj.scale = size
    bpy.ops.object.transform_apply(scale=True)
    obj.data.materials.append(mat)
    bevel = obj.modifiers.new("bevel", "BEVEL")
    bevel.width = 0.01
    bevel.segments = 2
    bpy.ops.object.modifier_apply(modifier=bevel.name)
    return obj


def main(out):
    bpy.ops.wm.read_factory_settings(use_empty=True)
    paint = material("boat-hull", (0.80, 0.82, 0.84), 0.35)
    stripe = material("boat-stripe", (0.05, 0.16, 0.38), 0.4)
    wood = material("boat-wood", (0.45, 0.30, 0.17), 0.6)
    motor = material("boat-motor", (0.06, 0.06, 0.07), 0.3, 0.6)

    hull = hull_mesh("boat", STATIONS, AROUND, solid=True)
    hull.data.materials.append(paint)
    hull.data.materials.append(stripe)
    # The rim and the top 12 cm of the outside in the stripe's blue.
    for poly in hull.data.polygons:
        if poly.center.z > gunwale(2.0 * poly.center.y / LENGTH) - 0.12:
            poly.material_index = 1
    parts = [hull]
    # Three thwarts across the hull, a hand under the gunwale.
    for s in (-0.55, 0.0, 0.45):
        y = 0.5 * LENGTH * s
        width = 2.0 * half_beam(s) * 0.93
        parts.append(box(f"thwart{s}", (width, 0.28, 0.035), (0.0, y, gunwale(s) - 0.18), wood))
    # The outboard motor on the transom: its head, its leg, its skeg.
    y = -0.5 * LENGTH - 0.18
    parts.append(box("motor-head", (0.34, 0.42, 0.42), (0.0, y, 0.18), motor))
    parts.append(box("motor-leg", (0.1, 0.16, 0.7), (0.0, y - 0.04, -0.35), motor))
    parts.append(box("motor-skeg", (0.04, 0.3, 0.18), (0.0, y - 0.04, -0.72), motor))
    bpy.ops.object.select_all(action="DESELECT")
    for p in parts:
        p.select_set(True)
    bpy.context.view_layer.objects.active = hull
    bpy.ops.object.join()
    bpy.ops.object.shade_auto_smooth(angle=math.radians(35))

    buoyancy = hull_mesh("boat-buoyancy", 16, 6, solid=False)
    buoyancy.data.materials.append(paint)

    bpy.ops.object.select_all(action="SELECT")
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
    main(args[0] if args else "boat.glb")
