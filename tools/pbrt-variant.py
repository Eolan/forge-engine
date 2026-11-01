# D-083's calibration: a pbrt-v4 scene rewritten so that pbrt-v4 renders what Forge and Falcor draw
# from tools/pbrt-to-gltf.py's glTF of it, the third reference beside Falcor's (tools/pbrt-render.sh
# runs it). Plain Python 3, the standard library alone:
#
#   python -I tools/pbrt-variant.py IN.pbrt OUT_DIR [--clay] [--flat] [--opaque]
#
# Writes OUT_DIR/scene.pbrt (and OUT_DIR/models/ with --flat); the input's files are only read.
# Every relative "string filename" of a shape, a texture or a light (a sky map) is made absolute,
# to the input's folder, so the scene renders from OUT_DIR.
# - --clay: every MakeNamedMaterial a 50 % grey diffuse (the comparisons' clay, D-083).
# - --flat: each plymesh rewritten without its shading normals (nx, ny, nz), so pbrt-v4 shades
#   with the faces' own normals, as the converter's glTF ("each face its own normal"). Bitterli's
#   bedroom curtains otherwise let a shadow ray through their folds (D-083, 2026-10-31). The
#   inline trianglemeshes (the rooms' area lights, flat quads) keep theirs: without them pbrt-v4
#   would face a light by its winding.
# - --opaque: "diffusetransmission" made "diffuse" with its reflectance: the converter leaves the
#   transmission out (the bedroom's curtains, the kitchen's blind), so Forge and Falcor draw them
#   opaque.
import os
import sys

args = [a for a in sys.argv[1:] if not a.startswith("--")]
flags = {a for a in sys.argv[1:] if a.startswith("--")}
if len(args) != 2 or flags - {"--clay", "--flat", "--opaque"}:
    sys.exit("usage: pbrt-variant.py IN.pbrt OUT_DIR [--clay] [--flat] [--opaque]")
src, out_dir = args
base = os.path.dirname(os.path.abspath(src))
os.makedirs(out_dir, exist_ok=True)

SIZES = {"char": 1, "uchar": 1, "int8": 1, "uint8": 1, "short": 2, "ushort": 2, "int16": 2,
         "uint16": 2, "int": 4, "uint": 4, "int32": 4, "uint32": 4, "float": 4, "float32": 4,
         "double": 8, "float64": 8}


def flat_ply(path, dst):
    """Copies a binary little-endian PLY without its vertices' nx, ny, nz. False if it has none."""
    with open(path, "rb") as f:
        data = f.read()
    end = data.index(b"end_header") + len(b"end_header")
    end = data.index(b"\n", end) + 1
    header = data[:end].decode("ascii").splitlines()
    if "format binary_little_endian 1.0" not in header:
        sys.exit(f"{path}: not a binary little-endian PLY")
    vertices, props, element = 0, [], None
    for line in header:
        words = line.split()
        if words[:1] == ["element"]:
            element = words[1]
            if element == "vertex":
                vertices = int(words[2])
        elif words[:1] == ["property"] and element == "vertex":
            if words[1] == "list":
                sys.exit(f"{path}: a list among the vertex properties")
            props.append((words[2], SIZES[words[1]]))
    names = [n for n, _ in props]
    if "nx" not in names:
        return False
    stride = sum(s for _, s in props)
    keep = [(o, s) for (n, s), o in zip(props, [sum(s for _, s in props[:i]) for i in range(len(props))])
            if n not in ("nx", "ny", "nz")]
    body = data[end:]
    rows = bytearray()
    for v in range(vertices):
        row = body[v * stride:(v + 1) * stride]
        for o, s in keep:
            rows += row[o:o + s]
    lines = [line for line in header if not (line.startswith("property") and line.split()[-1] in ("nx", "ny", "nz"))]
    with open(dst, "wb") as f:
        f.write(("\n".join(lines) + "\n").encode("ascii"))
        f.write(rows)
        f.write(body[vertices * stride:])
    return True


out, skipping, materials, flattened = [], False, 0, 0
in_ply, directive = False, None
for line in open(src, encoding="utf-8"):
    stripped = line.strip()
    if "--clay" in flags and line.startswith("MakeNamedMaterial"):
        name = line.split('"')[1]
        out.append(f'MakeNamedMaterial "{name}" "string type" [ "diffuse" ] "rgb reflectance" [ 0.5 0.5 0.5 ]\n')
        skipping, materials = True, materials + 1
        continue
    if skipping and (line.startswith((" ", "\t")) or not stripped):
        continue
    skipping = False
    if "--opaque" in flags and '"diffusetransmission"' in line:
        line = line.replace('"diffusetransmission"', '"diffuse"')
    if "--opaque" in flags and stripped.startswith('"rgb transmittance"'):
        continue
    if stripped[:1].isupper():  # a directive; its parameters follow it
        directive = stripped.split()[0]
        in_ply = directive == "Shape" and '"plymesh"' in line
    if '"string filename"' in line and directive in ("Shape", "Texture", "LightSource"):
        rel = line.split('"')[3]
        path = rel if os.path.isabs(rel) else os.path.join(base, rel)
        if in_ply and "--flat" in flags and rel.endswith(".ply"):
            dst = os.path.join(out_dir, "models", os.path.basename(rel))
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            if flat_ply(path, dst):
                path, flattened = dst, flattened + 1
        line = line.replace(f'"{rel}"', '"' + path.replace("\\", "/") + '"')
    out.append(line)
with open(os.path.join(out_dir, "scene.pbrt"), "w", encoding="utf-8") as f:
    f.writelines(out)
print(f"wrote {os.path.join(out_dir, 'scene.pbrt')}: {materials} clay materials, {flattened} meshes flat")
