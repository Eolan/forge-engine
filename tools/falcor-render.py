# Mogwai (Falcor 9): renders a scene with Falcor's PathTracer, accumulated, to a linear EXR and
# Falcor's tone-mapped PNG, for D-083's calibration. Run by tools/falcor-render.sh, which sets the
# diffuse model and the GPU lock. Set by the environment:
#   BISTRO_SCENE     the .pyscene to load (the Bistro's BistroExterior.pyscene, the chart's,
#                    tools/pbrt-to-gltf.py's classroom and Cornell box)
#   BISTRO_VIEW      "x,y,z,yaw,pitch" in Forge's --view convention (degrees; the scenes share axes)
#   BISTRO_FOV       vertical field of view, degrees
#   BISTRO_SIZE      "width,height"
#   BISTRO_SPP       frames accumulated, one sample each
#   BISTRO_BOUNCES   the most bounces (Falcor's 3 by default; 16 for the comparisons)
#   BISTRO_LIGHTS    "env" (the scene's sky alone, as the Bistro's readme says for ray tracing),
#                    "all" (with the scene's analytic lights) or "emissive" (its glow alone: no
#                    sky; a black sky must be off, as Falcor's importance sampling of it is NaN)
#   BISTRO_MATERIAL  "name:field=value,...;...": factors set by hand
#   BISTRO_EMITTER_STRENGTH  tools/pbrt-to-gltf.py's emitters' strength (Falcor's glTF import
#                    leaves KHR_materials_emissive_strength out): the scene's brightest light
#   BISTRO_CLAY, BISTRO_MATTE, BISTRO_DIFFUSE, BISTRO_NOEMIT: the surfaces simplified (below)
#   BISTRO_ALBEDO    1: the first hit's diffuse albedo (Falcor's G-buffer) in place of the light
#   BISTRO_CLAY_CUTOUTS  1: clay keeps a cut-out's base colour map (and so its alpha)
#   BISTRO_OUT       output folder and BISTRO_NAME base file name
import math
import os
import time
from falcor import *

env = os.environ
x, y, z, yaw, pitch = (float(v) for v in env["BISTRO_VIEW"].split(","))
fov = float(env.get("BISTRO_FOV", "36.22"))
width, height = (int(v) for v in env.get("BISTRO_SIZE", "1920,1061").split(","))
spp = int(env.get("BISTRO_SPP", "1024"))
lights = env.get("BISTRO_LIGHTS", "env")

g = RenderGraph("Reference")
# The camera jitter: 1,024 stratified positions (Falcor's most), so fine textures average over
# more than the default 16.
jitter = int(env.get("BISTRO_JITTER", "1024"))
if env.get("BISTRO_ALBEDO"):
    # BISTRO_ALBEDO: the diffuse albedo Falcor's materials give each pixel's first hit (its
    # G-buffer's `diffuseOpacity`, averaged over the jitter), NAME.AccumulatePass.output.0.exr,
    # to set beside Forge's `r.show.input=1` (D-083: the materials' gap). No light, no tone map.
    g.addPass(createPass("GBufferRT", {"samplePattern": "Stratified", "sampleCount": jitter, "useAlphaTest": True}), "GBufferRT")
    g.addPass(createPass("AccumulatePass", {"enabled": True, "precisionMode": "Single"}), "AccumulatePass")
    g.addEdge("GBufferRT.diffuseOpacity", "AccumulatePass.input")
    g.markOutput("AccumulatePass.output")
else:
    g.addPass(createPass("VBufferRT", {"samplePattern": "Stratified", "sampleCount": jitter, "useAlphaTest": True}), "VBufferRT")
    # BISTRO_BOUNCES: the most diffuse and specular bounces (Falcor defaults to 3, which drops
    # the light of longer paths: the shade between pale walls).
    bounces = int(env.get("BISTRO_BOUNCES", "3"))
    g.addPass(createPass("PathTracer", {"samplesPerPixel": 1, "maxSurfaceBounces": bounces, "maxDiffuseBounces": bounces, "maxSpecularBounces": bounces}), "PathTracer")
    g.addPass(createPass("AccumulatePass", {"enabled": True, "precisionMode": "Single"}), "AccumulatePass")
    g.addPass(createPass("ToneMapper", {"autoExposure": False, "exposureCompensation": float(env.get("BISTRO_EXPOSURE", "0"))}), "ToneMapper")
    g.addEdge("VBufferRT.vbuffer", "PathTracer.vbuffer")
    g.addEdge("VBufferRT.viewW", "PathTracer.viewW")
    g.addEdge("VBufferRT.mvec", "PathTracer.mvec")
    g.addEdge("PathTracer.color", "AccumulatePass.input")
    g.addEdge("AccumulatePass.output", "ToneMapper.src")
    g.markOutput("ToneMapper.dst")
    g.markOutput("AccumulatePass.output")

m.loadScene(env["BISTRO_SCENE"])
m.addGraph(g)
m.resizeFrameBuffer(width, height)
s = m.scene
s.animated = False
# BISTRO_MATERIAL "name:field=value,field=value": a material's factors changed for an
# experiment, its "Specular" texture cleared so they hold.
for spec in filter(None, env.get("BISTRO_MATERIAL", "").split(";")):
    name, fields = spec.split(":")
    mat = s.getMaterial(name)
    # Falcor merges materials that are alike at load (the bedroom's two lamps' emitters): the
    # first name stands for them all.
    if mat is None:
        print("MATERIAL", name, "not in the scene (merged into an alike one)")
        continue
    mat.clearTexture(MaterialTextureSlot.Specular)
    for field in fields.split(","):
        key, value = field.split("=")
        if key == "white":
            mat.clearTexture(MaterialTextureSlot.BaseColor)
            mat.baseColor = float4(1, 1, 1, 1)
            continue
        setattr(mat, key, float(value))
    print("MATERIAL", name, mat.roughness, mat.metallic)
s.camera.animated = False
# The emitters as Falcor read them (a glTF's KHR_materials_emissive_strength among them).
for mat in s.materials:
    try:
        if mat.emissiveFactor > 0 and max(mat.emissiveColor.x, mat.emissiveColor.y, mat.emissiveColor.z) > 0:
            print("EMISSIVE", mat.name, mat.emissiveColor, mat.emissiveFactor)
    except AttributeError:
        pass
# BISTRO_EMITTER_STRENGTH: tools/pbrt-to-gltf.py's emitters' strength (their colour is their light
# over it): Falcor's glTF import leaves KHR_materials_emissive_strength out.
if "BISTRO_EMITTER_STRENGTH" in env:
    for mat in s.materials:
        if mat.name.startswith("emitter-"):
            mat.emissiveFactor = float(env["BISTRO_EMITTER_STRENGTH"])
            print("EMITTER", mat.name, mat.emissiveColor, mat.emissiveFactor, "base", mat.baseColor)
# BISTRO_CLAY: every material a 50 % grey diffuse (no maps but the normals, no emission, no
# reflection: an index of refraction of 1, as BISTRO_DIFFUSE), but tools/pbrt-to-gltf.py's
# emitters (`emitter-`): a room's lights, kept. Before 2026-10-31 clay kept Falcor's dielectric
# layer (an index of 1.5 over the grey): 0.10 to 0.15 stops more light in the rooms at night than
# pbrt-v4's diffuse and Forge's clay.
if env.get("BISTRO_CLAY"):
    for mat in s.materials:
        if mat.name.startswith("emitter-"):
            continue
        try:
            # BISTRO_CLAY_CUTOUTS=1: a cut-out (its alpha in its base colour map, `AlphaMode.Mask`:
            # the leaves) keeps its map, so it stays cut out (in its own colours); cleared, the
            # leaf cards draw whole.
            cutout = env.get("BISTRO_CLAY_CUTOUTS") and mat.alphaMode == AlphaMode.Mask
            for slot in [MaterialTextureSlot.Specular, MaterialTextureSlot.Emissive] + ([] if cutout else [MaterialTextureSlot.BaseColor]):
                mat.clearTexture(slot)
            if not cutout:
                mat.baseColor = float4(0.5, 0.5, 0.5, 1)
            mat.roughness = 1.0
            mat.metallic = 0.0
            mat.emissiveFactor = 0.0
            mat.indexOfRefraction = 1.0
        except AttributeError:
            pass
# BISTRO_MATTE: every material a rough dielectric (its "Specular" map cleared, roughness 1, no
# metal), its base colour and normal maps kept: the diffuse light alone, with the real albedos.
if env.get("BISTRO_MATTE"):
    for mat in s.materials:
        try:
            mat.clearTexture(MaterialTextureSlot.Specular)
            mat.roughness = 1.0
            mat.metallic = 0.0
        except AttributeError:
            pass
# BISTRO_DIFFUSE: as BISTRO_MATTE, without any reflection (an index of refraction of 1): the
# diffuse light alone.
if env.get("BISTRO_DIFFUSE"):
    for mat in s.materials:
        try:
            mat.indexOfRefraction = 1.0
        except AttributeError:
            pass
# BISTRO_NOEMIT: every material's emission off (to take the emissive light alone, by difference).
if env.get("BISTRO_NOEMIT"):
    for mat in s.materials:
        try:
            mat.emissiveFactor = 0.0
        except AttributeError:
            pass
# BISTRO_ENVROT: the photograph turned round the vertical by this many degrees (an experiment).
# BISTRO_ENVSCALE: the photograph's intensity in place of the pyscene's 10 (an experiment).
if "BISTRO_ENVSCALE" in env:
    s.envMap.intensity = float(env["BISTRO_ENVSCALE"])
for l in s.lights:
    print("LIGHT", l.name, getattr(l, "direction", None), l.intensity)
if "BISTRO_ENVROT" in env:
    s.envMap.rotation = float3(0, float(env["BISTRO_ENVROT"]), 0)
    print("ENVROT", s.envMap.rotation)
settings = s.renderSettings
settings.useAnalyticLights = lights == "all"
# BISTRO_LIGHTS "emissive": the night, the emissive surfaces alone (no sky, no analytic lights).
settings.useEnvLight = lights != "emissive"
if lights == "emissive":
    s.envMap.intensity = 0.0
s.renderSettings = settings
print("LIGHTS", [l.name for l in s.lights], "analytic", settings.useAnalyticLights)

# Forge's fly camera: yaw 0 looks down -z, 90 down -x; pitch up.
cy, sy = math.cos(math.radians(yaw)), math.sin(math.radians(yaw))
cp, sp = math.cos(math.radians(pitch)), math.sin(math.radians(pitch))
forward = (-sy * cp, sp, -cy * cp)
c = s.camera
c.position = float3(x, y, z)
c.target = float3(x + forward[0], y + forward[1], z + forward[2])
c.up = float3(0, 1, 0)
c.focalLength = 0.5 * c.frameHeight / math.tan(math.radians(fov) / 2)
print("CAMERA", c.position, c.target, c.focalLength, c.frameHeight)

m.clock.pause()
m.frameCapture.outputDir = env["BISTRO_OUT"]
m.frameCapture.baseFilename = env["BISTRO_NAME"]
start = time.time()
for i in range(spp):
    m.renderFrame()
print("RENDERED", spp, "frames in", round(time.time() - start, 1), "s")
m.frameCapture.capture()
exit()
