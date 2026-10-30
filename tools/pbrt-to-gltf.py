# Blender script (D-083): a pbrt-v4 scene to glTF for Forge and Falcor, with its sky, so that both
# renderers read the same files. The subset Benedikt Bitterli's pbrt-v4 conversions use: `plymesh`
# and `trianglemesh` shapes, `Transform` blocks, named and anonymous materials (diffuse, coated
# diffuse, conductor, dielectric, diffuse transmission), image textures, diffuse area lights, a
# distant light and an image infinite light.
#
#   blender -b --factory-startup --python tools/pbrt-to-gltf.py -- IN.pbrt OUT_DIR
#
# Writes into OUT_DIR, each file named after OUT_DIR's last part (NAME):
# - NAME.gltf, NAME.bin: every shape's triangles in world space (transforms baked, the winding
#   turned where one mirrors), its normals where the file has them (else each face's own), its UVs;
#   one glTF material per pbrt material, double-sided (pbrt's surfaces reflect on both sides), its
#   factors and base colour map (written as PNG beside it); the camera.
# - NAME.view.txt: the camera as Forge's `--view=x,y,z,yaw,pitch --fov`; NAME.camera.json, the
#   lab's start there.
# - NAME.sky.hdr: the infinite light's map (pbrt-v4's equal-area square) in Falcor's latitude-
#   longitude layout, 2048 x 1024, the distant light drawn in as a disc of 0.5° radius whose
#   texels sum to its irradiance: the one sky both renderers read (as tools/material-chart.py's).
# - NAME.pyscene: Falcor's scene, the glTF under that sky.
# - NAME.lights.json: the lights as read (the area lights are the emitting materials).
#
# The scene's own files stay where they are; only the PNGs are new. Keep OUT_DIR out of a folder
# named glTF under assets/external/ (crates/forge-geom/tests/external_models.rs loads those).
import bpy, json, math, os, re, struct, sys
import numpy as np

argv = sys.argv[sys.argv.index("--") + 1:]
src, out = os.path.abspath(argv[0]), os.path.abspath(argv[1])
base = os.path.dirname(src)
name = os.path.basename(out.rstrip("\\/"))
os.makedirs(out, exist_ok=True)

# ------------------------------------------------------------------ the file's tokens ---

def tokens(text):
    for m in re.finditer(r'"([^"]*)"|(\[)|(\])|#[^\n]*|([^\s\[\]"#]+)', text):
        if m.group(1) is not None:
            yield ("str", m.group(1))
        elif m.group(2):
            yield ("[", None)
        elif m.group(3):
            yield ("]", None)
        elif m.group(4) is not None:
            yield ("word", m.group(4))

PARAM_TYPES = {"integer", "float", "point2", "vector2", "point3", "vector3", "normal", "normal3",
               "point", "vector", "bool", "string", "rgb", "spectrum", "blackbody", "texture"}

def parse(text):
    """The directives: (name, [leading strings], {param name: (type, [values])})."""
    out, cur = [], None
    it = list(tokens(text))
    i = 0
    while i < len(it):
        kind, value = it[i]
        if kind == "word" and re.match(r"[A-Z][A-Za-z]*$", value):
            cur = (value, [], {}, [])
            out.append(cur)
            i += 1
            continue
        if cur is None:
            raise ValueError(f"a value before any directive: {value}")
        words = value.split() if kind == "str" else []
        if len(words) == 2 and words[0] in PARAM_TYPES:
            ptype, pname = words
            i += 1
            values = []
            if it[i][0] == "[":
                i += 1
                while it[i][0] != "]":
                    values.append(it[i][1])
                    i += 1
                i += 1
            else:
                values.append(it[i][1])
                i += 1
            cur[2][pname] = (ptype, values)
            continue
        if kind == "[":
            # A directive's own numbers (Transform [ ... ]).
            i += 1
            while it[i][0] != "]":
                cur[3].append(float(it[i][1]))
                i += 1
            i += 1
            continue
        cur[1].append(value)
        i += 1
    return out

def matrix(values):
    # pbrt lists a matrix by columns: the last four numbers are the translation.
    return np.array(values, dtype=np.float64).reshape(4, 4).T

# ------------------------------------------------------------------------- meshes ---

PLY_TYPES = {"float": "<f4", "float32": "<f4", "double": "<f8", "uchar": "u1", "uint8": "u1",
             "char": "i1", "int8": "i1", "short": "<i2", "int16": "<i2", "ushort": "<u2",
             "uint16": "<u2", "int": "<i4", "int32": "<i4", "uint": "<u4", "uint32": "<u4"}

def read_ply(path):
    data = open(path, "rb").read()
    end = data.index(b"end_header") + len(b"end_header")
    end = data.index(b"\n", end) + 1
    header = data[:end].decode("ascii").splitlines()
    if "format binary_little_endian 1.0" not in header:
        raise ValueError(f"{path}: only binary little-endian PLY is read")
    elements, current = [], None
    for line in header:
        words = line.split()
        if words[:1] == ["element"]:
            current = {"name": words[1], "count": int(words[2]), "props": []}
            elements.append(current)
        elif words[:1] == ["property"]:
            current["props"].append(words[1:])
    at = end
    vertices = faces = None
    for el in elements:
        if el["name"] == "vertex":
            dtype = np.dtype([(p[1], PLY_TYPES[p[0]]) for p in el["props"]])
            vertices = np.frombuffer(data, dtype=dtype, count=el["count"], offset=at)
            at += dtype.itemsize * el["count"]
        elif el["name"] == "face":
            (_, count_t, index_t, _) = el["props"][0]
            ct, it_ = np.dtype(PLY_TYPES[count_t]), np.dtype(PLY_TYPES[index_t])
            first = int(np.frombuffer(data, dtype=ct, count=1, offset=at)[0])
            row = np.dtype([("n", ct), ("i", it_, (first,))])
            rows = np.frombuffer(data, dtype=row, count=el["count"], offset=at) \
                if at + row.itemsize * el["count"] <= len(data) else None
            if rows is not None and np.all(rows["n"] == first):
                polys = [list(r) for r in rows["i"]]
                at += row.itemsize * el["count"]
            else:
                polys = []
                for _ in range(el["count"]):
                    n = int(np.frombuffer(data, dtype=ct, count=1, offset=at)[0])
                    at += ct.itemsize
                    polys.append(list(np.frombuffer(data, dtype=it_, count=n, offset=at)))
                    at += it_.itemsize * n
            faces = polys
        else:
            raise ValueError(f"{path}: element {el['name']} not read")
    names = vertices.dtype.names
    p = np.stack([vertices["x"], vertices["y"], vertices["z"]], 1).astype(np.float64)
    n = np.stack([vertices["nx"], vertices["ny"], vertices["nz"]], 1).astype(np.float64) \
        if "nx" in names else None
    uv = None
    for a, b in (("u", "v"), ("s", "t"), ("texture_u", "texture_v")):
        if a in names:
            uv = np.stack([vertices[a], vertices[b]], 1).astype(np.float64)
    tris = []
    for f in faces:
        # A quad as pbrt-v4 splits a planar one; larger polygons as fans.
        for k in range(1, len(f) - 1):
            tris.append((f[0], f[k], f[k + 1]))
    return p, n, uv, np.array(tris, dtype=np.int64).reshape(-1, 3)

def baked(p, n, uv, tris, m):
    """The mesh in world space: `m` applied, normals by its inverse transpose, the winding turned
    where it mirrors, flat normals where the file had none."""
    p = p @ m[:3, :3].T + m[:3, 3]
    if np.linalg.det(m[:3, :3]) < 0:
        tris = tris[:, ::-1]
    if n is None:
        # Each face its own normal (pbrt's geometric one): the corners split.
        p = p[tris].reshape(-1, 3)
        uv = uv[tris].reshape(-1, 2) if uv is not None else None
        fn = np.cross(p[1::3] - p[0::3], p[2::3] - p[0::3])
        fn /= np.maximum(np.linalg.norm(fn, axis=1, keepdims=True), 1e-20)
        n = np.repeat(fn, 3, axis=0)
        tris = np.arange(len(p)).reshape(-1, 3)
    else:
        n = n @ np.linalg.inv(m[:3, :3])
        n /= np.maximum(np.linalg.norm(n, axis=1, keepdims=True), 1e-20)
    return p, n, uv, tris

# ---------------------------------------------------------------------- materials ---

# F0 of the named metals pbrt-v4's conductors take ("metal-Cu-eta"), linear sRGB at normal incidence.
METALS = {"Al": (0.913, 0.922, 0.924), "Ag": (0.972, 0.960, 0.915), "Au": (1.0, 0.766, 0.336),
          "Cu": (0.955, 0.638, 0.538), "CuZn": (0.95, 0.85, 0.55), "MgO": (0.06, 0.06, 0.06),
          "TiO2": (0.25, 0.25, 0.25)}

def f0_from(eta, k):
    eta, k = np.array(eta, dtype=np.float64), np.array(k, dtype=np.float64)
    return list(((eta - 1) ** 2 + k ** 2) / ((eta + 1) ** 2 + k ** 2))

def roughness(params):
    """glTF's roughness (its square the GGX alpha) from pbrt's."""
    r = None
    if "roughness" in params:
        r = float(params["roughness"][1][0])
    elif "uroughness" in params:
        r = 0.5 * (float(params["uroughness"][1][0]) + float(params["vroughness"][1][0]))
    if r is None:
        return 0.0
    remap = params.get("remaproughness", ("bool", ["true"]))[1][0] in ("true", True)
    alpha = math.sqrt(r) if remap else r
    return min(1.0, math.sqrt(max(alpha, 0.0)))

def gltf_material(mname, kind, params, texture_index):
    m = {"name": mname, "doubleSided": True,
         "pbrMetallicRoughness": {"metallicFactor": 0.0, "roughnessFactor": 1.0},
         "extras": {"pbrt": kind}}
    pbr = m["pbrMetallicRoughness"]
    refl = params.get("reflectance")
    if refl and refl[0] == "rgb":
        pbr["baseColorFactor"] = [float(v) for v in refl[1]] + [1.0]
    elif refl and refl[0] == "texture":
        index = texture_index(refl[1][0])
        if index is not None:
            pbr["baseColorTexture"] = {"index": index}
    if kind == "coateddiffuse":
        pbr["roughnessFactor"] = roughness(params)
    elif kind == "conductor":
        pbr["metallicFactor"] = 1.0
        pbr["roughnessFactor"] = roughness(params)
        eta, k = params.get("eta"), params.get("k")
        if eta and eta[0] == "rgb" and k and k[0] == "rgb":
            color = f0_from([float(v) for v in eta[1]], [float(v) for v in k[1]])
        elif eta and eta[0] == "spectrum" and isinstance(eta[1][0], str):
            metal = eta[1][0].split("-")[1]
            color = METALS.get(metal, (0.9, 0.9, 0.9))
        else:
            color = METALS["Cu"]  # pbrt-v4's default conductor
        pbr["baseColorFactor"] = [min(1.0, float(c)) for c in color] + [1.0]
    elif kind == "dielectric":
        pbr["baseColorFactor"] = [1.0, 1.0, 1.0, 1.0]
        pbr["roughnessFactor"] = roughness(params)
        m.setdefault("extensions", {})["KHR_materials_transmission"] = {"transmissionFactor": 1.0}
        eta = float(params.get("eta", ("float", [1.5]))[1][0])
        m["extensions"]["KHR_materials_ior"] = {"ior": eta}
    elif kind == "diffusetransmission":
        t = params.get("transmittance")
        m["extras"]["transmittance"] = [float(v) for v in t[1]] if t else None
    return m

# ---------------------------------------------------------------------------- sky ---

def read_pfm(path):
    with open(path, "rb") as f:
        kind = f.readline().strip()
        w, h = (int(v) for v in f.readline().split())
        scale = float(f.readline())
        channels = 3 if kind == b"PF" else 1
        data = np.frombuffer(f.read(), dtype="<f4" if scale < 0 else ">f4")
    img = data.reshape(h, w, channels).astype(np.float64) * abs(scale)
    # PFM's rows run bottom to top; pbrt reads them top first.
    return img[::-1]

def sphere_to_square(d):
    """pbrt-v4's EqualAreaSphereToSquare (Clarberg's octahedral equal-area map), vectorised."""
    x, y, z = np.abs(d[..., 0]), np.abs(d[..., 1]), np.abs(d[..., 2])
    r = np.sqrt(np.maximum(1.0 - z, 0.0))
    a, b = np.maximum(x, y), np.minimum(x, y)
    b = np.where(a == 0, 0.0, b / np.where(a == 0, 1.0, a))
    phi = np.arctan(b) * 2.0 / math.pi
    phi = np.where(x < y, 1.0 - phi, phi)
    v = phi * r
    u = r - v
    neg = d[..., 2] < 0
    u, v = np.where(neg, 1.0 - v, u), np.where(neg, 1.0 - u, v)
    u = np.copysign(u, d[..., 0])
    v = np.copysign(v, d[..., 1])
    return 0.5 * (u + 1.0), 0.5 * (v + 1.0)

def square_to_sphere(u, v):
    """pbrt-v4's EqualAreaSquareToSphere, for the round trip below."""
    u, v = 2.0 * u - 1.0, 2.0 * v - 1.0
    up, vp = np.abs(u), np.abs(v)
    signed = 1.0 - (up + vp)
    r = 1.0 - np.abs(signed)
    phi = np.where(r == 0, 1.0, (vp - up) / np.where(r == 0, 1.0, r) + 1.0) * math.pi / 4.0
    z = np.copysign(1.0 - r * r, signed)
    cos_phi = np.copysign(np.cos(phi), u)
    sin_phi = np.copysign(np.sin(phi), v)
    s = r * np.sqrt(np.maximum(2.0 - r * r, 0.0))
    return np.stack([cos_phi * s, sin_phi * s, z], -1)

def bilinear(img, u, v):
    h, w = img.shape[:2]
    x, y = u * w - 0.5, v * h - 0.5
    x0, y0 = np.floor(x).astype(int), np.floor(y).astype(int)
    fx, fy = (x - x0)[..., None], (y - y0)[..., None]
    def at(yy, xx):
        return img[np.clip(yy, 0, h - 1), np.clip(xx, 0, w - 1)]
    return (at(y0, x0) * (1 - fx) * (1 - fy) + at(y0, x0 + 1) * fx * (1 - fy)
            + at(y0 + 1, x0) * (1 - fx) * fy + at(y0 + 1, x0 + 1) * fx * fy)

def write_hdr(path, rgb):
    # A Radiance RGBE file, rows top to bottom, uncompressed (tools/material-chart.py's writer).
    top = np.max(rgb, axis=-1)
    mantissa, exponent = np.frexp(top)
    scale = np.where(top > 1e-32, mantissa * 256.0 / np.where(top > 1e-32, top, 1.0), 0.0)
    m = np.clip(rgb * scale[..., None], 0, 255).astype(np.uint8)
    e = np.where(top > 1e-32, exponent + 128, 0).astype(np.uint8)
    rgbe = np.concatenate([m, e[..., None]], axis=-1)
    with open(path, "wb") as f:
        f.write(f"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y {rgb.shape[0]} +X {rgb.shape[1]}\n".encode())
        f.write(rgbe.tobytes())

# ------------------------------------------------------------------------- the scene ---

directives = parse(open(src, encoding="utf-8").read())
ctm = np.eye(4)
stack = []
material = None
area = None
camera_from_world = None
fov = None
xres, yres = 1280, 720
textures, named = {}, {}
anonymous = 0
shapes = []  # (positions, normals, uvs, triangles, material name, area light or None)
lights = {"distant": [], "infinite": [], "area": []}

def push():
    stack.append((ctm.copy(), material, area))

def pop():
    global ctm, material, area
    ctm, material, area = stack.pop()

for d, lead, params, nums in directives:
    if d == "Transform":
        ctm = matrix(nums)
    elif d == "Camera":
        camera_from_world = ctm.copy()
        fov = float(params.get("fov", ("float", [90]))[1][0])
    elif d == "Film":
        xres = int(params.get("xresolution", ("integer", [1280]))[1][0])
        yres = int(params.get("yresolution", ("integer", [720]))[1][0])
    elif d == "WorldBegin":
        ctm = np.eye(4)
    elif d in ("TransformBegin", "AttributeBegin"):
        push()
    elif d in ("TransformEnd", "AttributeEnd"):
        pop()
    elif d == "Texture":
        textures[lead[0]] = os.path.join(base, params["filename"][1][0])
    elif d == "MakeNamedMaterial":
        named[lead[0]] = (params["type"][1][0], params)
    elif d == "NamedMaterial":
        material = lead[0]
    elif d == "Material":
        anonymous += 1
        key = f"material-{anonymous}"
        named[key] = (lead[0], params)
        material = key
    elif d == "AreaLightSource":
        area = [float(v) for v in params["L"][1]]
    elif d == "LightSource":
        kind = lead[0]
        if kind == "distant":
            fr = np.array([float(v) for v in params.get("from", ("point3", [0, 0, 0]))[1]])
            to = np.array([float(v) for v in params.get("to", ("point3", [0, 0, 1]))[1]])
            towards = ctm[:3, :3] @ (fr - to)
            lights["distant"].append({
                "towards_light": list(towards / np.linalg.norm(towards)),
                "L": [float(v) for v in params["L"][1]],
            })
        elif kind == "infinite":
            lights["infinite"].append({
                "filename": os.path.join(base, params["filename"][1][0]),
                "light_to_world": ctm.tolist(),
                "scale": float(params.get("scale", ("float", [1]))[1][0]),
            })
        else:
            raise ValueError(f"light {kind} not read")
    elif d == "Shape":
        kind = lead[0]
        if kind == "plymesh":
            p, n, uv, tris = read_ply(os.path.join(base, params["filename"][1][0]))
        elif kind == "trianglemesh":
            p = np.array([float(v) for v in params["P"][1]]).reshape(-1, 3)
            n = np.array([float(v) for v in params["N"][1]]).reshape(-1, 3) if "N" in params else None
            uv = np.array([float(v) for v in params["uv"][1]]).reshape(-1, 2) if "uv" in params else None
            tris = np.array([int(float(v)) for v in params["indices"][1]]).reshape(-1, 3)
        else:
            raise ValueError(f"shape {kind} not read")
        shapes.append((*baked(p, n, uv, tris, ctm), material, area))
    elif d in ("Integrator", "Sampler", "PixelFilter", "WorldEnd"):
        pass
    else:
        raise ValueError(f"directive {d} not read")

# The materials: one per pbrt material a shape uses, one per area light's emission.
gltf = {"asset": {"version": "2.0", "generator": "Forge tools/pbrt-to-gltf.py"},
        "scene": 0, "scenes": [{"nodes": []}], "nodes": [], "meshes": [], "materials": [],
        "accessors": [], "bufferViews": [], "buffers": [], "images": [], "textures": [],
        "samplers": [{"magFilter": 9729, "minFilter": 9987, "wrapS": 10497, "wrapT": 10497}],
        "extensionsUsed": []}
images = {}

def texture_index(tname):
    path = textures.get(tname)
    if path is None:
        return None
    if path not in images:
        png = os.path.splitext(os.path.basename(path))[0] + ".png"
        img = bpy.data.images.load(path)
        if img.size[0] == 0 or len(img.pixels) == 0:
            raise ValueError(f"{path} does not read")
        img.filepath_raw = os.path.join(out, png)
        img.file_format = "PNG"
        img.save()
        gltf["images"].append({"uri": png})
        gltf["textures"].append({"source": len(gltf["images"]) - 1, "sampler": 0})
        images[path] = len(gltf["textures"]) - 1
    return images[path]

material_index = {}
for _, _, _, _, mname, emission in shapes:
    key = (mname, tuple(emission) if emission else None)
    if key in material_index:
        continue
    if emission:
        top = max(emission)
        m = {"name": f"emitter-{len(material_index)}", "doubleSided": False,
             "pbrMetallicRoughness": {"baseColorFactor": [0, 0, 0, 1], "metallicFactor": 0.0,
                                      "roughnessFactor": 1.0},
             "emissiveFactor": [e / top for e in emission],
             "extensions": {"KHR_materials_emissive_strength": {"emissiveStrength": top}}}
        lights["area"].append({"material": m["name"], "L": emission})
    else:
        kind, params = named[mname]
        m = gltf_material(mname, kind, params, texture_index)
    for ext in m.get("extensions", {}):
        if ext not in gltf["extensionsUsed"]:
            gltf["extensionsUsed"].append(ext)
    gltf["materials"].append(m)
    material_index[key] = len(gltf["materials"]) - 1

blob = bytearray()

def add(array, target=None):
    view = {"buffer": 0, "byteOffset": len(blob), "byteLength": array.nbytes}
    if target:
        view["target"] = target
    blob.extend(array.tobytes())
    while len(blob) % 4:
        blob.append(0)
    gltf["bufferViews"].append(view)
    return len(gltf["bufferViews"]) - 1

def accessor(array, kind, component, target=None, bounds=False):
    a = {"bufferView": add(array, target), "componentType": component, "count": len(array),
         "type": kind}
    if bounds:
        a["min"] = [float(v) for v in array.min(axis=0)]
        a["max"] = [float(v) for v in array.max(axis=0)]
    gltf["accessors"].append(a)
    return len(gltf["accessors"]) - 1

triangles = 0
for i, (p, n, uv, tris, mname, emission) in enumerate(shapes):
    attributes = {
        "POSITION": accessor(p.astype("<f4"), "VEC3", 5126, 34962, bounds=True),
        "NORMAL": accessor(n.astype("<f4"), "VEC3", 5126, 34962),
    }
    if uv is not None:
        # glTF's V runs down the image, pbrt's up.
        attributes["TEXCOORD_0"] = accessor(
            np.stack([uv[:, 0], 1.0 - uv[:, 1]], 1).astype("<f4"), "VEC2", 5126, 34962)
    key = (mname, tuple(emission) if emission else None)
    gltf["meshes"].append({"name": f"shape-{i}", "primitives": [{
        "attributes": attributes,
        "indices": accessor(tris.reshape(-1).astype("<u4"), "SCALAR", 5125, 34963),
        "material": material_index[key]}]})
    gltf["nodes"].append({"name": f"shape-{i}", "mesh": i})
    gltf["scenes"][0]["nodes"].append(i)
    triangles += len(tris)

# The camera: pbrt's looks down its +z, x right, y up; glTF's down its -z.
world_from_camera = np.linalg.inv(camera_from_world)
right, up, forward = (world_from_camera[:3, k] for k in range(3))
right, up, forward = (v / np.linalg.norm(v) for v in (right, up, forward))
position = world_from_camera[:3, 3]
rot = np.stack([right, up, -forward], 1)
# The rotation's quaternion (x, y, z, w).
t = np.trace(rot)
if t > 0:
    s = math.sqrt(t + 1.0) * 2
    q = [(rot[2, 1] - rot[1, 2]) / s, (rot[0, 2] - rot[2, 0]) / s, (rot[1, 0] - rot[0, 1]) / s, 0.25 * s]
else:
    k = int(np.argmax(np.diag(rot)))
    i1, i2 = (k + 1) % 3, (k + 2) % 3
    s = math.sqrt(1.0 + rot[k, k] - rot[i1, i1] - rot[i2, i2]) * 2
    q = [0.0, 0.0, 0.0, (rot[i2, i1] - rot[i1, i2]) / s]
    q[k] = 0.25 * s
    q[i1] = (rot[i1, k] + rot[k, i1]) / s
    q[i2] = (rot[i2, k] + rot[k, i2]) / s
# pbrt's fov is the shorter side's.
vfov = fov if yres <= xres else 2 * math.degrees(math.atan(math.tan(math.radians(fov) / 2) * yres / xres))
gltf["cameras"] = [{"type": "perspective", "perspective": {
    "yfov": math.radians(vfov), "aspectRatio": xres / yres, "znear": 0.01}}]
gltf["nodes"].append({"name": "camera", "camera": 0, "translation": [float(v) for v in position],
                      "rotation": [float(v) for v in q]})
gltf["scenes"][0]["nodes"].append(len(gltf["nodes"]) - 1)

with open(os.path.join(out, f"{name}.bin"), "wb") as f:
    f.write(blob)
gltf["buffers"] = [{"uri": f"{name}.bin", "byteLength": len(blob)}]
for empty in ("images", "textures", "extensionsUsed"):
    if not gltf[empty]:
        del gltf[empty]
if "textures" not in gltf:
    del gltf["samplers"]
with open(os.path.join(out, f"{name}.gltf"), "w") as f:
    json.dump(gltf, f, indent=1)

# Forge's fly camera: yaw 0 looks down -z, 90 down -x; pitch up; no roll.
pitch = math.degrees(math.asin(max(-1.0, min(1.0, forward[1]))))
yaw = math.degrees(math.atan2(-forward[0], -forward[2]))
roll = math.degrees(math.asin(max(-1.0, min(1.0, right[1]))))
view = (f"--view={position[0]:.4f},{position[1]:.4f},{position[2]:.4f},{yaw:.3f},{pitch:.3f} "
        f"--fov {vfov:.4f}")
# The lab's start (demos/physics-lab's `CameraPath`, one frame): a rotation taking -Y to the
# camera's forward, as Blender's cameras come through the axis change.
axis = np.cross([0.0, -1.0, 0.0], forward)
half = np.array([*axis, 1.0 + np.dot([0.0, -1.0, 0.0], forward)])
half /= np.linalg.norm(half)
with open(os.path.join(out, f"{name}.camera.json"), "w") as f:
    json.dump({"frames": [[*(float(v) for v in position), *(float(v) for v in half)]]}, f)
with open(os.path.join(out, f"{name}.view.txt"), "w") as f:
    f.write(view + f"\n# {xres} x {yres}; roll {roll:.3f} degrees (Forge's view has none)\n")

# The sky: the infinite light's map turned into the world, in Falcor's layout
# (u = (1 + atan2(x, -z) / pi) / 2, v = acos(y) / pi), the distant light drawn in.
W, H = 2048, 1024
u = (np.arange(W) + 0.5) / W
v = (np.arange(H) + 0.5) / H
phi = (2.0 * u - 1.0) * math.pi
theta = v * math.pi
dirs = np.stack([np.sin(phi)[None, :] * np.sin(theta)[:, None],
                 np.cos(theta)[:, None] * np.ones((1, W)),
                 -np.cos(phi)[None, :] * np.sin(theta)[:, None]], -1)
sky = np.zeros((H, W, 3))
for inf in lights["infinite"]:
    img = read_pfm(inf["filename"])
    world_from_light = np.array(inf["light_to_world"])[:3, :3]
    local = dirs @ np.linalg.inv(world_from_light).T
    local /= np.linalg.norm(local, axis=-1, keepdims=True)
    # The map's own round trip, a check on the two functions.
    su, sv = sphere_to_square(local)
    back = square_to_sphere(su, sv)
    assert np.max(np.abs(back - local)) < 1e-4, np.max(np.abs(back - local))
    # pbrt's (u, v): x along a row, y down the rows.
    sky += bilinear(img[..., :3], su, sv) * inf["scale"]
solid = (2.0 * math.pi / W) * (math.pi / H) * np.sin(theta)[:, None] * np.ones((1, W))
for dist in lights["distant"]:
    towards = np.array(dist["towards_light"])
    disc = (dirs @ towards) > math.cos(math.radians(0.5))
    for c in range(3):
        sky[..., c] = np.where(disc, dist["L"][c] / (solid * disc).sum(), sky[..., c])
write_hdr(os.path.join(out, f"{name}.sky.hdr"), sky)

with open(os.path.join(out, f"{name}.pyscene"), "w") as f:
    f.write(f"# {name} (tools/pbrt-to-gltf.py) under its sky, its sun drawn in.\n"
            f'sceneBuilder.importScene("{name}.gltf")\n'
            f'sceneBuilder.envMap = EnvMap("{name}.sky.hdr")\n'
            "sceneBuilder.envMap.intensity = 1\n")
with open(os.path.join(out, f"{name}.lights.json"), "w") as f:
    json.dump(lights, f, indent=1)
print(f"PBRT {name}: {len(shapes)} shapes, {triangles} triangles, {len(gltf['materials'])} "
      f"materials, {len(images)} maps; {view}")
