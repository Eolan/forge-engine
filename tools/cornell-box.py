# The Cornell box as Falcor's test scene builds it (`media/test_scenes/cornell_box.pyscene`), written
# as a pbrt-v4 scene: the same box for pbrt-v4, Forge and Falcor (D-083's calibration), the first
# two from this file and Falcor through tools/pbrt-to-gltf.py's glTF and pyscene.
#
#   python tools/cornell-box.py OUT_DIR [SCALE]
#
# Writes OUT_DIR/cornell.pbrt. The box ten times Falcor's by default (SCALE 10: a room 5.5 m
# wide, as Forge's probes and ray offsets are set for metres; the light's radiance stays, so the
# image is the same at every scale). Its walls, floor, ceiling, two blocks and light as Falcor's
# quads and cubes place them (a unit quad in xz facing +y, a unit cube, both centred), every
# surface Lambert's diffuse at Falcor's base colours (its roughness-0.5 highlight left out: the
# diffuse light alone is compared), the light a quad facing down, emitting Falcor's
# 5 x (17, 12, 4) from its front. Falcor's camera (0, 0.28, 1.2) looking down -z, 35 mm on a
# 24 mm frame, at 1600 x 900.
import math
import os
import sys

out = sys.argv[1]
scale = float(sys.argv[2]) if len(sys.argv) > 2 else 10.0
os.makedirs(out, exist_ok=True)


def mat_mul(a, b):
    return [[sum(a[i][k] * b[k][j] for k in range(4)) for j in range(4)] for i in range(4)]


def scaling(x, y, z):
    return [[x, 0, 0, 0], [0, y, 0, 0], [0, 0, z, 0], [0, 0, 0, 1]]


def translation(x, y, z):
    return [[1, 0, 0, x], [0, 1, 0, y], [0, 0, 1, z], [0, 0, 0, 1]]


def rotation(axis, radians):
    c, s = math.cos(radians), math.sin(radians)
    if axis == "x":
        return [[1, 0, 0, 0], [0, c, -s, 0], [0, s, c, 0], [0, 0, 0, 1]]
    if axis == "y":
        return [[c, 0, s, 0], [0, 1, 0, 0], [-s, 0, c, 0], [0, 0, 0, 1]]
    return [[c, -s, 0, 0], [s, c, 0, 0], [0, 0, 1, 0], [0, 0, 0, 1]]


def place(scale_xyz, translate=(0, 0, 0), axis="x", radians=0.0):
    """Falcor's Transform: scale, then rotate, then translate (Falcor's units, times `scale`)."""
    t = [v * scale for v in translate]
    m = mat_mul(rotation(axis, radians), scaling(*[v * scale for v in scale_xyz]))
    return mat_mul(translation(*t), m)


def apply(m, p):
    return [sum(m[i][k] * (p[k] if k < 3 else 1.0) for k in range(4)) for i in range(3)]


# A unit quad in xz facing +y, counter-clockwise seen from +y; a unit cube facing out. Each face
# has UVs over 0..1: without them Falcor builds no tangent frame and shades NaN.
QUAD = ([(-0.5, 0, -0.5), (-0.5, 0, 0.5), (0.5, 0, 0.5), (0.5, 0, -0.5)], [(0, 1, 2), (0, 2, 3)],
        [(0, 0), (0, 1), (1, 1), (1, 0)])
CUBE_FACES = [
    ((0, 1, 0), (1, 0, 0), (0, 0, -1)),
    ((0, -1, 0), (1, 0, 0), (0, 0, 1)),
    ((1, 0, 0), (0, 0, -1), (0, 1, 0)),
    ((-1, 0, 0), (0, 0, 1), (0, 1, 0)),
    ((0, 0, 1), (1, 0, 0), (0, 1, 0)),
    ((0, 0, -1), (-1, 0, 0), (0, 1, 0)),
]


def cube():
    points, tris, uvs = [], [], []
    for n, u, v in CUBE_FACES:
        base = len(points)
        for a, b in [(-1, -1), (1, -1), (1, 1), (-1, 1)]:
            points.append(tuple(0.5 * n[k] + 0.5 * a * u[k] + 0.5 * b * v[k] for k in range(3)))
            uvs.append((0.5 * (a + 1), 0.5 * (b + 1)))
        tris += [(base, base + 1, base + 2), (base, base + 2, base + 3)]
    return points, tris, uvs


def shape(m, mesh):
    points, tris, uvs = mesh
    p = [apply(m, q) for q in points]
    # Facing as placed: each triangle's winding kept (no placement mirrors).
    ps = " ".join("%.6f %.6f %.6f" % tuple(v) for v in p)
    ix = " ".join("%d %d %d" % t for t in tris)
    uv = " ".join("%g %g" % t for t in uvs)
    return f'Shape "trianglemesh" "point3 P" [ {ps} ] "point2 uv" [ {uv} ] "integer indices" [ {ix} ]\n'


WHITE = (0.725, 0.71, 0.68)
pi = math.pi
lines = []
# The camera, as tools/pbrt-to-gltf.py reads pbrt's (+z forward, x right, y up): world from
# camera's columns right, up, forward and its place; the file takes camera from world, its 16
# numbers column by column.
eye = (0.0, 0.28 * scale, 1.2 * scale)
right, up, forward = (1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, -1.0)
world_from_camera = [[right[i], up[i], forward[i], eye[i]] for i in range(3)] + [[0, 0, 0, 1]]
r = [[world_from_camera[j][i] for j in range(3)] for i in range(3)]
t = [-sum(r[i][k] * eye[k] for k in range(3)) for i in range(3)]
camera_from_world = [r[0] + [t[0]], r[1] + [t[1]], r[2] + [t[2]], [0, 0, 0, 1]]
vfov = 2 * math.degrees(math.atan(12.0 / 35.0))
lines.append('Integrator "path" "integer maxdepth" [ 16 ]\n')
lines.append("Transform [ " + " ".join("%.8g" % camera_from_world[i][j] for j in range(4) for i in range(4)) + " ]\n")
lines.append('Sampler "sobol" "integer pixelsamples" [ 4096 ]\n')
lines.append('PixelFilter "box"\n')
lines.append('Film "rgb" "string filename" [ "cornell.exr" ] "integer xresolution" [ 1600 ] "integer yresolution" [ 900 ]\n')
lines.append(f'Camera "perspective" "float fov" [ {vfov:.6f} ]\n')
lines.append("WorldBegin\n")
surfaces = [
    ("floor", WHITE, place((0.55, 1.0, 0.56)), QUAD),
    ("ceiling", WHITE, place((0.55, 1.0, 0.56), (0, 0.55, 0), "x", pi), QUAD),
    ("back", WHITE, place((0.55, 1.0, 0.56), (0, 0.275, -0.275), "x", pi / 2), QUAD),
    ("left", (0.63, 0.065, 0.05), place((0.55, 1.0, 0.56), (-0.275, 0.275, 0), "z", -pi / 2), QUAD),
    ("right", (0.14, 0.45, 0.091), place((0.55, 1.0, 0.56), (0.275, 0.275, 0), "z", pi / 2), QUAD),
    ("large-box", WHITE, place((0.165, 0.33, 0.165), (-0.093, 0.165, -0.071), "y", -1.27), cube()),
    ("small-box", WHITE, place((0.165, 0.165, 0.165), (0.09, 0.0825, 0.111), "y", -0.29), cube()),
]
for name, colour, m, mesh in surfaces:
    lines.append(f'MakeNamedMaterial "{name}" "string type" [ "diffuse" ] "rgb reflectance" [ {colour[0]} {colour[1]} {colour[2]} ]\n')
for name, colour, m, mesh in surfaces:
    lines.append("AttributeBegin\n")
    lines.append(f'NamedMaterial "{name}"\n')
    lines.append(shape(m, mesh))
    lines.append("AttributeEnd\n")
# The light: Falcor's quad of 0.13 at the ceiling, turned to face down, emitting from its front.
lines.append("AttributeBegin\n")
lines.append('AreaLightSource "diffuse" "rgb L" [ 85 60 20 ]\n')
lines.append('Material "diffuse" "rgb reflectance" [ 0 0 0 ]\n')
lines.append(shape(place((0.13, 0.13, 0.13), (0, 0.549, 0), "x", pi), QUAD))
lines.append("AttributeEnd\n")
with open(os.path.join(out, "cornell.pbrt"), "w", encoding="utf-8") as f:
    f.writelines(lines)
print("wrote", os.path.join(out, "cornell.pbrt"))
