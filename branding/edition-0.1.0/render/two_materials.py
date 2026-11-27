# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           branding/edition-0.1.0/render/two_materials.py
# Responsibility:   Give the two chevrons of the mark two metals and render the scene.
# Dependencies:     sys, bpy
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Give the two chevrons of the mark two metals and render the scene.

Run inside Blender with a scene written by logo3d:

  blender -b SCENE.blend -P two_materials.py -- OUT.png SAMPLES WIDTHxHEIGHT

The scene is the one that the program logo3d writes with the option --blend
for the file svg/mark-black.svg prepared for three dimensions.

The image depends on the version of Blender. The seed of the sampling of
Cycles is fixed, so that two runs of one version give the same pixels.
"""
import sys

import bpy

RENDER_SEED = 0

out, samples, size = sys.argv[sys.argv.index("--") + 1 :][:3]
width, height = (int(v) for v in size.split("x"))

logo = bpy.data.objects["logo"]
base = logo.material_slots[0].material


def metal(name, colour, roughness):
    material = base.copy()
    material.name = name
    node = next(n for n in material.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    node.inputs["Base Color"].default_value = (*colour, 1.0)
    node.inputs["Metallic"].default_value = 1.0
    node.inputs["Roughness"].default_value = roughness
    return material


silver = metal("logo_silver", (0.80, 0.82, 0.85), 0.16)
gold = metal("logo_gold", (0.86, 0.66, 0.36), 0.20)
logo.data.materials.clear()
logo.data.materials.append(silver)
logo.data.materials.append(gold)
xs = [v.co.x for v in logo.data.vertices]
centre = (min(xs) + max(xs)) / 2
for polygon in logo.data.polygons:
    polygon.material_index = 1 if polygon.center.x > centre else 0

scene = bpy.context.scene
scene.render.resolution_x, scene.render.resolution_y = width, height
scene.render.resolution_percentage = 100
scene.cycles.samples = int(samples)
scene.cycles.seed = RENDER_SEED
scene.cycles.use_animated_seed = False
scene.render.filepath = out
bpy.ops.render.render(write_still=True)

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
