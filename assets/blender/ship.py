# The physics lab's spaceship (`--lab space`, the owner's ask of 2026-10-03: "spaceship looks
# like a more sci-fi spaceship"), modelled in Blender from code and exported as glTF:
#
#   blender --background --factory-startup --python assets/blender/ship.py -- OUT.glb
#
# A small faceted fighter-shuttle, 16.8 m long with a 13 m span: a wide, flat, angular hull
# with a dark canopy and glowing strips along its sides, swept wings with blades turned down
# at their tips, two engine nacelles at the wing roots and a main engine in the tail, their
# nozzles open on glowing throats, two canted fins, panels and stripes, running lights. Three
# meshes and three points go out:
#   ship         the ship, its origin at its middle (its weight's), the nose along +Y;
#   ship-flame   an engine's flame, a cone from its origin at a nozzle's mouth backwards
#                (drawn once per engine, slid out of its nozzle with the throttle);
#   ship-shell   a coarse closed hull of the hull, the nacelles and the wings, for collision;
#   nozzle-left, nozzle-middle, nozzle-right   the nozzles' mouths (empties).
# Blender's +Y (the nose) becomes glTF's -Z, Forge's forward; +Z up becomes +Y. A material's
# emission strength is Forge's emissive, in units of a white surface facing the sun.

import math
import sys

import bmesh
import bpy

LENGTH = 16.0
NOSE_Y = 8.0
NACELLE_X = 2.6
NACELLE_Z = -0.3
NACELLE_RADIUS = 0.68
NACELLE_FRONT = 0.4
NACELLE_BACK = -7.3
# The nozzles: open frustums from the nacelle's (or the tail's) end, their mouths this far back.
NOZZLE_LENGTH = 0.9
TAIL_NOZZLE_Z = 0.15
TAIL_NOZZLE_RADIUS = 0.6


def material(name, color, roughness, metallic=0.0, emission=0.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    bsdf = m.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    if emission > 0.0:
        bsdf.inputs["Emission Color"].default_value = (*color, 1.0)
        bsdf.inputs["Emission Strength"].default_value = emission
    return m


def link(name, mesh, mat):
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    obj.data.materials.append(mat)
    return obj


def section(t):
    """The hull's half width, top and bottom at `t` (0 at the nose, 1 at the tail)."""
    # A pointed nose widening fast, the widest a little behind the middle, a cut-off tail.
    if t < 0.55:
        s = math.sin(t / 0.55 * math.pi / 2) ** 0.8
    else:
        s = 1.0 - 0.25 * ((t - 0.55) / 0.45) ** 2
    return 0.15 + 1.55 * s, 0.1 + 0.8 * s, 0.08 + 0.5 * s


def hull_z(t):
    """The hull's middle line: the nose droops a little, the tail lifts."""
    return -0.25 * (1.0 - t) ** 3 + 0.1 * t


def hull(mat):
    """An angular loft: eight-sided sections, flat on top and underneath, from the nose back."""
    mesh = bpy.data.meshes.new("hull")
    bm = bmesh.new()
    stations = 24
    rings = []
    for i in range(stations + 1):
        t = i / stations
        y = NOSE_Y - LENGTH * t
        w, top, bottom = section(t)
        z = hull_z(t)
        ring = [
            (w, 0.0),
            (0.74 * w, 0.75 * top),
            (0.34 * w, top),
            (-0.34 * w, top),
            (-0.74 * w, 0.75 * top),
            (-w, 0.0),
            (-0.62 * w, -bottom),
            (0.62 * w, -bottom),
        ]
        rings.append([bm.verts.new((x, y, z + h)) for x, h in ring])
    for a, b in zip(rings, rings[1:]):
        n = len(a)
        for k in range(n):
            bm.faces.new((a[k], b[k], b[(k + 1) % n], a[(k + 1) % n]))
    bm.faces.new(list(reversed(rings[0])))
    bm.faces.new(rings[-1])
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(mesh)
    bm.free()
    return link("hull", mesh, mat)


def box(name, size, at, mat, bevel=0.02, rotation=(0.0, 0.0, 0.0)):
    """A box in the world's frame (its vertices where they stand, its origin at 0)."""
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=at, rotation=rotation)
    obj = bpy.context.active_object
    obj.name = name
    obj.scale = size
    bpy.ops.object.transform_apply(location=True, scale=True, rotation=True)
    obj.data.materials.append(mat)
    if bevel > 0.0:
        mod = obj.modifiers.new("bevel", "BEVEL")
        mod.width = bevel
        mod.segments = 2
        bpy.ops.object.modifier_apply(modifier=mod.name)
    return obj


def along_y(name, radius, depth, y, x, z, mat, vertices=16, radius2=None, open_ends=False):
    """A cylinder (or a frustum from `radius` at its front to `radius2` at its back) along the
    ship, its middle at `y`; open-ended ones get a wall's thickness, so they show inside."""
    fill = "NOTHING" if open_ends else "NGON"
    bpy.ops.mesh.primitive_cone_add(
        vertices=vertices, radius1=radius2 if radius2 is not None else radius, radius2=radius,
        depth=depth, location=(x, y, z), rotation=(-math.pi / 2, 0.0, 0.0), end_fill_type=fill,
    )
    obj = bpy.context.active_object
    obj.name = name
    bpy.ops.object.transform_apply(location=True, rotation=True)
    obj.data.materials.append(mat)
    if open_ends:
        mod = obj.modifiers.new("wall", "SOLIDIFY")
        mod.thickness = 0.06
        bpy.ops.object.modifier_apply(modifier=mod.name)
    return obj


def wing(side, mat):
    """A swept slab from the hull's side to its tip, angled down, its chord shrinking out."""
    root_y, root_chord, tip_chord, span, sweep, droop = -1.6, 6.0, 1.7, 5.0, 3.4, 0.5
    thickness = 0.22
    x0 = side * 1.4
    obj = box(f"wing{side}", (span, root_chord, thickness), (x0 + side * span / 2, root_y, -0.05), mat, bevel=0.06)
    for v in obj.data.vertices:
        f = abs(v.co.x - x0) / span  # 0 at the root, 1 at the tip
        chord = root_chord + (tip_chord - root_chord) * f
        middle = root_y - sweep * f
        v.co.y = middle + (v.co.y - root_y) * chord / root_chord
        v.co.z -= droop * f
    return obj


def blade(side, mat):
    """A blade turned down and out at a wing's tip, swept back."""
    tip_x, tip_y, tip_z = side * 6.4, -5.0, -0.55
    obj = box(f"blade{side}", (0.12, 1.6, 1.1), (tip_x, tip_y, tip_z - 0.55), mat, bevel=0.03)
    for v in obj.data.vertices:
        f = (tip_z - v.co.z) / 1.1  # 0 at the wing, 1 at the blade's end
        v.co.y = tip_y - 0.9 * f + (v.co.y - tip_y) * (1.0 - 0.45 * f)
        v.co.x += side * 0.7 * f
    return obj


def fin(side, mat):
    """A canted fin on the tail's top, swept back."""
    obj = box(f"fin{side}", (0.13, 2.2, 1.2), (0.0, -5.8, 0.9 + 0.6), mat, bevel=0.04)
    for v in obj.data.vertices:
        f = (v.co.z - 0.9) / 1.2  # 0 at its root, 1 at its tip
        v.co.y = -5.8 - 1.1 * f + (v.co.y + 5.8) * (1.0 - 0.5 * f)
    obj.location = (side * 0.25, 0.0, 0.0)
    obj.rotation_euler = (0.0, side * math.radians(34.0), 0.0)
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.transform_apply(location=True, rotation=True)
    obj.select_set(False)
    return obj


def engine(name, x, z, radius, back, mats, nacelle=True):
    """A nacelle (or none, for the tail's engine) from the front back to `back`, an accent ring,
    an open nozzle flaring out behind it and its glowing throat; returns the parts and where
    the nozzle's mouth is."""
    hull_white, accent, metal, glow = mats
    parts = []
    if nacelle:
        length = NACELLE_FRONT - back
        parts.append(along_y(f"{name}-nacelle", radius, length, (NACELLE_FRONT + back) / 2, x, z, hull_white))
        # A rounded nose cone ahead of it, a dark band where they meet.
        parts.append(along_y(f"{name}-cone", radius * 0.3, 1.1, NACELLE_FRONT + 0.55, x, z, hull_white, radius2=radius))
        parts.append(along_y(f"{name}-band", radius * 1.02, 0.12, NACELLE_FRONT, x, z, metal))
        parts.append(along_y(f"{name}-ring", radius * 1.07, 0.35, back + 1.5, x, z, accent))
    mouth = back - NOZZLE_LENGTH
    parts.append(along_y(f"{name}-nozzle", radius * 0.85, NOZZLE_LENGTH, back - NOZZLE_LENGTH / 2, x, z, metal, radius2=radius * 1.02, open_ends=True))
    # The throat, a little inside the nozzle.
    parts.append(along_y(f"{name}-throat", radius * 0.84, 0.04, back - 0.25, x, z, glow))
    return parts, (x, mouth, z)


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
    hull_white = material("ship-hull", (0.72, 0.74, 0.78), 0.35, 0.55)
    panel = material("ship-panel", (0.2, 0.22, 0.26), 0.5, 0.4)
    accent = material("ship-accent", (0.90, 0.33, 0.05), 0.4, 0.1)
    glass = material("ship-glass", (0.02, 0.05, 0.09), 0.05, 0.0)
    metal = material("ship-metal", (0.16, 0.16, 0.18), 0.3, 0.9)
    glow = material("ship-glow", (0.35, 0.8, 1.0), 0.5, 0.0, emission=6.0)
    strip = material("ship-strip", (0.3, 0.85, 1.0), 0.5, 0.0, emission=3.0)
    red = material("ship-light-red", (1.0, 0.08, 0.05), 0.5, 0.0, emission=5.0)
    green = material("ship-light-green", (0.1, 1.0, 0.2), 0.5, 0.0, emission=5.0)
    white = material("ship-light-white", (1.0, 1.0, 0.95), 0.5, 0.0, emission=5.0)
    flame_mat = material("ship-flame", (0.45, 0.8, 1.0), 0.5, 0.0, emission=9.0)
    mats = (hull_white, accent, metal, glow)

    parts = [hull(hull_white)]
    # The canopy: a dark faceted bubble over the hull's front third.
    canopy = box("canopy", (1.3, 3.2, 0.8), (0.0, 3.6, 0.5), glass, bevel=0.3)
    for v in canopy.data.vertices:
        f = (v.co.y - 2.0) / 3.2  # 0 at its back, 1 at its front
        v.co.x *= 1.0 - 0.45 * f
        v.co.z = 0.5 + (v.co.z - 0.5) * (1.0 - 0.5 * f) + 0.12 * (1.0 - f)
    parts.append(canopy)
    # A dorsal spine behind the canopy and panels along it.
    parts.append(box("spine", (0.8, 6.0, 0.3), (0.0, -1.6, 0.9), panel, bevel=0.08))
    for k, y in enumerate((0.8, -0.7, -2.2, -3.7)):
        parts.append(box(f"panel{k}", (0.56, 1.1, 0.1), (0.0, y, 1.08), hull_white, bevel=0.03))
    # Glowing strips along the hull's widest line, and intakes over them in front of the wings.
    for side in (-1.0, 1.0):
        parts.append(box(f"strip{side}", (0.07, 7.4, 0.07), (side * 1.69, -2.0, 0.04), strip, bevel=0.0))
        parts.append(box(f"intake{side}", (0.36, 2.4, 0.5), (side * 1.62, 1.3, 0.28), panel, bevel=0.06))
    # The wings, their stripes, their blades and their tips' lights.
    for side in (-1.0, 1.0):
        parts.append(wing(side, hull_white))
        # Across the wing behind its middle line, following its sweep and its droop; a little
        # thicker than the wing, so it shows on both faces.
        stripe = box(f"stripe{side}", (3.4, 0.45, 0.24), (side * 3.6, -2.75, -0.1), accent, bevel=0.03)
        for v in stripe.data.vertices:
            f = (abs(v.co.x) - 1.9) / 3.4
            v.co.y -= 2.3 * f
            v.co.z -= 0.34 * f
        parts.append(stripe)
        parts.append(blade(side, panel))
        bpy.ops.mesh.primitive_uv_sphere_add(radius=0.14, segments=12, ring_count=8, location=(side * 6.5, -4.2, -0.55))
        tip = bpy.context.active_object
        tip.name = f"tip-light{side}"
        tip.data.materials.append(red if side < 0 else green)
        parts.append(tip)
    # The engines: a nacelle under each wing's root, the main one in the tail.
    mouths = {}
    for side, name in ((-1.0, "left"), (1.0, "right")):
        p, mouths[name] = engine(name, side * NACELLE_X, NACELLE_Z, NACELLE_RADIUS, NACELLE_BACK, mats)
        parts.extend(p)
    p, mouths["middle"] = engine("middle", 0.0, TAIL_NOZZLE_Z, TAIL_NOZZLE_RADIUS, NOSE_Y - LENGTH + 0.02, mats, nacelle=False)
    parts.extend(p)
    for name, at in mouths.items():
        empty = bpy.data.objects.new(f"nozzle-{name}", None)
        empty.location = at
        bpy.context.collection.objects.link(empty)
    # The fins and the tail light on the spine's end.
    for side in (-1.0, 1.0):
        parts.append(fin(side, hull_white))
    bpy.ops.mesh.primitive_uv_sphere_add(radius=0.12, segments=12, ring_count=8, location=(0.0, -4.7, 1.1))
    tail = bpy.context.active_object
    tail.name = "tail-light"
    tail.data.materials.append(white)
    parts.append(tail)
    ship = join(parts, "ship")
    bpy.ops.object.shade_auto_smooth(angle=math.radians(30))

    # A flame: a cone from its origin backwards (-Y), wide at the nozzle.
    along_y("ship-flame", 0.03, 4.0, -2.0, 0.0, 0.0, flame_mat, radius2=None)
    flame = bpy.context.active_object
    for v in flame.data.vertices:
        # Wide (0.42 m) at the origin, a point 4 m back.
        f = -v.co.y / 4.0
        r = math.hypot(v.co.x, v.co.z)
        if r > 1e-6:
            scale = (0.42 * (1.0 - f) + 0.03 * f) / r
            v.co.x *= scale
            v.co.z *= scale
    bpy.ops.object.shade_auto_smooth(angle=math.radians(60))

    # The collision hull: points round the hull, the nacelles and the wings' tips (their hull is
    # taken), one closed mesh.
    shell = box("ship-shell", (3.4, LENGTH - 0.6, 1.6), (0.0, 0.0, 0.1), metal, bevel=0.0)
    wings = box("shell-wings", (12.8, 3.4, 0.6), (0.0, -3.8, -0.4), metal, bevel=0.0)
    engines = box(
        "shell-engines",
        (2 * NACELLE_X + 2 * NACELLE_RADIUS, NACELLE_FRONT - NACELLE_BACK + NOZZLE_LENGTH, 2 * NACELLE_RADIUS),
        (0.0, (NACELLE_FRONT + NACELLE_BACK - NOZZLE_LENGTH) / 2, NACELLE_Z),
        metal,
        bevel=0.0,
    )
    shell = join([shell, wings, engines], "ship-shell")

    bpy.ops.object.select_all(action="DESELECT")
    for o in bpy.data.objects:
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
    main(args[0] if args else "ship.glb")
