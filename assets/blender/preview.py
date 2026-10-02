# Renders a glTF file on the CPU (Cycles) for a look at it:
#   blender --background --factory-startup --python preview.py -- IN.glb OUT.png
import math
import sys

import bpy
from mathutils import Vector

args = sys.argv[sys.argv.index("--") + 1 :]
bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=args[0])
for obj in bpy.data.objects:
    if obj.name.endswith("buoyancy"):
        obj.location.x += 3.0
scene = bpy.context.scene
scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.samples = 24
scene.render.resolution_x = 960
scene.render.resolution_y = 540
cam = bpy.data.objects.new("cam", bpy.data.cameras.new("cam"))
scene.collection.objects.link(cam)
cam.location = Vector((9.0, -9.5, 4.5))
direction = Vector((1.5, 0.0, -0.2)) - cam.location
cam.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
scene.camera = cam
sun = bpy.data.objects.new("sun", bpy.data.lights.new("sun", "SUN"))
sun.rotation_euler = (math.radians(50), 0, math.radians(30))
scene.collection.objects.link(sun)
world = bpy.data.worlds.new("w")
world.use_nodes = True
world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.5, 0.6, 0.75, 1)
scene.world = world
scene.render.filepath = args[1]
bpy.ops.render.render(write_still=True)
