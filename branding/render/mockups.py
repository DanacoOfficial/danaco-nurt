# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           branding/render/mockups.py
# Responsibility:   Renders the paper, phone and case mockup scenes of the mark in Blender from the images that prepare.py writes.
# Dependencies:     argparse, math, sys, pathlib, bpy, mathutils
# Author:           Danaco Official
# Version:          0.1.2, 2026-11-28
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Render the mockup scenes of the mark with Blender (Cycles, on the processor).

Three scenes use the images that prepare.py writes:

  paper   black paper with the two chevrons stamped in silver and gold foil
  phone   a phone that lies on a dark surface; its screen shows the icon
  case    an anodised metal case with the mark etched in two tones

Run inside Blender:

  blender -b --factory-startup -P mockups.py -- SCENE --assets DIR --hdri FILE
          -o OUT.png [--size 1000x1000] [--samples 96] [--threads 8]

The lighting is a studio HDRI that only lights the scene and is not seen by the
camera, a large softbox and a small rim light.
"""

import argparse
import math
import sys
from pathlib import Path

import bpy
from mathutils import Vector

SILVER = "#C3C8CE"
GOLD = "#D4AE6E"
GRAPHITE = "#0B0C0E"


def options():
    argv = sys.argv[sys.argv.index("--") + 1 :]
    parser = argparse.ArgumentParser()
    parser.add_argument("scene", choices=["paper", "phone", "case"])
    parser.add_argument("--assets", required=True, type=Path)
    parser.add_argument("--hdri", required=True, type=Path)
    parser.add_argument("-o", "--out", required=True, type=Path)
    parser.add_argument("--size", default="1000x1000")
    parser.add_argument("--samples", type=int, default=96)
    parser.add_argument("--threads", type=int, default=8)
    return parser.parse_args(argv)


def linear(colour):
    values = [int(colour[i : i + 2], 16) / 255 for i in (1, 3, 5)]
    return tuple(v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in values)


def link(tree, source, name, target, socket):
    tree.links.new(source.outputs[name], target.inputs[socket])


def new_material(name):
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    tree = material.node_tree
    for node in list(tree.nodes):
        tree.nodes.remove(node)
    return material, tree


def principled(tree, colour, metallic, roughness):
    node = tree.nodes.new("ShaderNodeBsdfPrincipled")
    node.inputs["Base Color"].default_value = (*linear(colour), 1.0)
    node.inputs["Metallic"].default_value = metallic
    node.inputs["Roughness"].default_value = roughness
    return node


def image_node(tree, path, data=True):
    node = tree.nodes.new("ShaderNodeTexImage")
    node.image = bpy.data.images.load(str(path))
    node.image.colorspace_settings.name = "Non-Color" if data else "sRGB"
    node.extension = "EXTEND"
    return node


def mapped(tree, width, height, reference):
    """Texture coordinates that keep an image of reference x reference units undistorted."""
    coordinates = tree.nodes.new("ShaderNodeTexCoord")
    mapping = tree.nodes.new("ShaderNodeMapping")
    mapping.inputs["Scale"].default_value = (width / reference, height / reference, 1.0)
    mapping.inputs["Location"].default_value = (0.5 - 0.5 * width / reference, 0.5 - 0.5 * height / reference, 0.0)
    link(tree, coordinates, "Generated", mapping, "Vector")
    return mapping


def rounded_slab(name, width, length, depth, corner, edge):
    """A slab with rounded corners in plan and a small rounding of the edges."""
    import bmesh

    mesh = bpy.data.meshes.new(name)
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    for vertex in bm.verts:
        vertex.co.x *= width
        vertex.co.y *= length
        vertex.co.z *= depth
    bm.to_mesh(mesh)
    bm.free()
    weights = mesh.attributes.new("bevel_weight_edge", "FLOAT", "EDGE")
    values = []
    for e in mesh.edges:
        a, b = (mesh.vertices[i].co for i in e.vertices)
        values.append(1.0 if abs(a.x - b.x) < 1e-6 and abs(a.y - b.y) < 1e-6 else 0.0)
    weights.data.foreach_set("value", values)
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    first = obj.modifiers.new("corner", "BEVEL")
    first.width, first.segments, first.limit_method = corner, 14, "WEIGHT"
    second = obj.modifiers.new("edge", "BEVEL")
    second.width, second.segments, second.limit_method = edge, 4, "ANGLE"
    second.angle_limit = math.radians(40)
    for polygon in mesh.polygons:
        polygon.use_smooth = True
    return obj


def plane(name, size):
    bpy.ops.mesh.primitive_plane_add(size=size)
    obj = bpy.context.object
    obj.name = name
    return obj


def world(hdri, strength):
    scene = bpy.context.scene
    scene.world = bpy.data.worlds.new("studio")
    scene.world.use_nodes = True
    tree = scene.world.node_tree
    for node in list(tree.nodes):
        tree.nodes.remove(node)
    environment = tree.nodes.new("ShaderNodeTexEnvironment")
    environment.image = bpy.data.images.load(str(hdri))
    lit = tree.nodes.new("ShaderNodeBackground")
    lit.inputs["Strength"].default_value = strength
    link(tree, environment, "Color", lit, "Color")
    dark = tree.nodes.new("ShaderNodeBackground")
    dark.inputs["Color"].default_value = (*linear(GRAPHITE), 1.0)
    dark.inputs["Strength"].default_value = 1.0
    path = tree.nodes.new("ShaderNodeLightPath")
    mix = tree.nodes.new("ShaderNodeMixShader")
    link(tree, path, "Is Camera Ray", mix, "Fac")
    tree.links.new(lit.outputs["Background"], mix.inputs[1])
    tree.links.new(dark.outputs["Background"], mix.inputs[2])
    output = tree.nodes.new("ShaderNodeOutputWorld")
    tree.links.new(mix.outputs["Shader"], output.inputs["Surface"])


def target():
    empty = bpy.data.objects.new("target", None)
    bpy.context.collection.objects.link(empty)
    return empty


def area(name, location, size, energy, aim):
    light = bpy.data.lights.new(name, "AREA")
    light.size, light.energy = size, energy
    obj = bpy.data.objects.new(name, light)
    obj.location = location
    bpy.context.collection.objects.link(obj)
    track = obj.constraints.new("TRACK_TO")
    track.target, track.track_axis, track.up_axis = aim, "TRACK_NEGATIVE_Z", "UP_Y"
    return obj


def camera(location, aim, focal, aperture):
    data = bpy.data.cameras.new("camera")
    data.lens = focal
    data.dof.use_dof = True
    data.dof.focus_object = aim
    data.dof.aperture_fstop = aperture
    obj = bpy.data.objects.new("camera", data)
    obj.location = location
    bpy.context.collection.objects.link(obj)
    track = obj.constraints.new("TRACK_TO")
    track.target, track.track_axis, track.up_axis = aim, "TRACK_NEGATIVE_Z", "UP_Y"
    bpy.context.scene.camera = obj


def reflector(location, size, aim, strength):
    """A panel with a gradient, seen only in the reflections of the surfaces."""
    obj = plane("reflector", size)
    obj.location = location
    track = obj.constraints.new("TRACK_TO")
    track.target, track.track_axis, track.up_axis = aim, "TRACK_Z", "UP_Y"
    material, tree = new_material("reflector")
    coordinates = tree.nodes.new("ShaderNodeTexCoord")
    split = tree.nodes.new("ShaderNodeSeparateXYZ")
    link(tree, coordinates, "Generated", split, "Vector")
    ramp = tree.nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].position = 0.0
    ramp.color_ramp.elements[0].color = (0.03, 0.03, 0.035, 1.0)
    ramp.color_ramp.elements[1].position = 1.0
    ramp.color_ramp.elements[1].color = (1.0, 1.0, 1.0, 1.0)
    middle = ramp.color_ramp.elements.new(0.45)
    middle.color = (0.55, 0.56, 0.6, 1.0)
    link(tree, split, "Y", ramp, "Fac")
    emission = tree.nodes.new("ShaderNodeEmission")
    emission.inputs["Strength"].default_value = strength
    link(tree, ramp, "Color", emission, "Color")
    output = tree.nodes.new("ShaderNodeOutputMaterial")
    tree.links.new(emission.outputs["Emission"], output.inputs["Surface"])
    obj.data.materials.append(material)
    return obj


def mirrored(camera_location, normal, aim, distance):
    """The place where a surface with this normal shows the reflection of the camera."""
    view = (aim.location - camera_location).normalized()
    reflected = view - 2.0 * view.dot(normal) * normal
    return aim.location + reflected * distance


def two_tone(tree, left, right, base, silver, gold):
    first, second = tree.nodes.new("ShaderNodeMixShader"), tree.nodes.new("ShaderNodeMixShader")
    link(tree, left, "Color", first, "Fac")
    tree.links.new(base.outputs["BSDF"], first.inputs[1])
    tree.links.new(silver.outputs["BSDF"], first.inputs[2])
    link(tree, right, "Color", second, "Fac")
    tree.links.new(first.outputs["Shader"], second.inputs[1])
    tree.links.new(gold.outputs["BSDF"], second.inputs[2])
    output = tree.nodes.new("ShaderNodeOutputMaterial")
    tree.links.new(second.outputs["Shader"], output.inputs["Surface"])


def paper(assets):
    surface = plane("paper", 9.0)
    surface.rotation_euler.z = math.radians(9)
    material, tree = new_material("paper")
    left, right = image_node(tree, assets / "mask-left.png"), image_node(tree, assets / "mask-right.png")
    relief = image_node(tree, assets / "bump.png")
    mapping = mapped(tree, 9.0, 9.0, 4.0)
    for node in (left, right, relief):
        link(tree, mapping, "Vector", node, "Vector")
    noise = tree.nodes.new("ShaderNodeTexNoise")
    noise.inputs["Scale"].default_value = 900.0
    noise.inputs["Detail"].default_value = 14.0
    noise.inputs["Roughness"].default_value = 0.7
    fibres = tree.nodes.new("ShaderNodeBump")
    fibres.inputs["Strength"].default_value = 0.5
    fibres.inputs["Distance"].default_value = 0.004
    link(tree, noise, "Fac", fibres, "Height")
    stamp = tree.nodes.new("ShaderNodeBump")
    stamp.invert = True
    stamp.inputs["Strength"].default_value = 1.0
    stamp.inputs["Distance"].default_value = 0.03
    link(tree, relief, "Color", stamp, "Height")
    link(tree, fibres, "Normal", stamp, "Normal")
    base = principled(tree, "#0B0C0E", 0.0, 0.9)
    silver = principled(tree, SILVER, 1.0, 0.16)
    gold = principled(tree, GOLD, 1.0, 0.18)
    for node in (base, silver, gold):
        link(tree, stamp, "Normal", node, "Normal")
    two_tone(tree, left, right, base, silver, gold)
    surface.data.materials.append(material)
    aim = target()
    place = Vector((1.2, -3.4, 3.4))
    camera(place, aim, 62.0, 5.0)
    reflector(mirrored(place, Vector((0, 0, 1)), aim, 5.5), 7.0, aim, 3.2)
    area("rim", Vector((3.6, 2.4, 1.6)), 1.5, 90.0, aim)
    return 0.12


def etched(assets):
    slab = rounded_slab("case", 3.4, 2.3, 0.16, 0.2, 0.012)
    slab.rotation_euler.z = math.radians(11)
    material, tree = new_material("case")
    left, right = image_node(tree, assets / "mask-left.png"), image_node(tree, assets / "mask-right.png")
    relief = image_node(tree, assets / "bump.png")
    mapping = mapped(tree, 3.4, 2.3, 2.6)
    for node in (left, right, relief):
        link(tree, mapping, "Vector", node, "Vector")
    grain = tree.nodes.new("ShaderNodeTexNoise")
    grain.inputs["Scale"].default_value = 380.0
    grain.inputs["Detail"].default_value = 8.0
    scale = tree.nodes.new("ShaderNodeMapping")
    scale.inputs["Scale"].default_value = (1.0, 0.04, 1.0)
    coordinates = tree.nodes.new("ShaderNodeTexCoord")
    link(tree, coordinates, "Object", scale, "Vector")
    link(tree, scale, "Vector", grain, "Vector")
    brushed = tree.nodes.new("ShaderNodeBump")
    brushed.inputs["Strength"].default_value = 0.05
    brushed.inputs["Distance"].default_value = 0.002
    link(tree, grain, "Fac", brushed, "Height")
    cut = tree.nodes.new("ShaderNodeBump")
    cut.invert = True
    cut.inputs["Strength"].default_value = 0.9
    cut.inputs["Distance"].default_value = 0.012
    link(tree, relief, "Color", cut, "Height")
    link(tree, brushed, "Normal", cut, "Normal")
    base = principled(tree, "#16181C", 1.0, 0.34)
    silver = principled(tree, "#C3C8CE", 1.0, 0.42)
    gold = principled(tree, "#D4AE6E", 1.0, 0.42)
    for node in (base, silver, gold):
        link(tree, cut, "Normal", node, "Normal")
    two_tone(tree, left, right, base, silver, gold)
    slab.data.materials.append(material)
    ground = plane("ground", 30.0)
    ground.location.z = -0.085
    ground_material, ground_tree = new_material("ground")
    ground_node = principled(ground_tree, "#09090B", 0.0, 0.6)
    ground_output = ground_tree.nodes.new("ShaderNodeOutputMaterial")
    ground_tree.links.new(ground_node.outputs["BSDF"], ground_output.inputs["Surface"])
    ground.data.materials.append(ground_material)
    aim = target()
    place = Vector((1.5, -3.5, 2.7))
    camera(place, aim, 62.0, 4.5)
    top = Vector((0.0, 0.0, 1.0))
    reflector(mirrored(place + Vector((0.0, 0.0, -0.08)), top, aim, 5.5), 7.0, aim, 3.0)
    area("rim", Vector((3.8, 2.0, 2.2)), 1.8, 200.0, aim)
    return 0.15


def phone(assets):
    body = rounded_slab("body", 1.0, 2.05, 0.1, 0.17, 0.014)
    metal, metal_tree = new_material("body")
    metal_node = principled(metal_tree, "#34373D", 1.0, 0.28)
    metal_output = metal_tree.nodes.new("ShaderNodeOutputMaterial")
    metal_tree.links.new(metal_node.outputs["BSDF"], metal_output.inputs["Surface"])
    body.data.materials.append(metal)
    glass = rounded_slab("screen", 0.94, 1.99, 0.012, 0.135, 0.002)
    glass.location.z = 0.05 + 0.004
    material, tree = new_material("screen")
    coordinates = tree.nodes.new("ShaderNodeTexCoord")
    picture = image_node(tree, assets / "screen.png", data=False)
    link(tree, coordinates, "Generated", picture, "Vector")
    shader = principled(tree, "#000000", 0.0, 0.05)
    shader.inputs["Emission Strength"].default_value = 1.1
    link(tree, picture, "Color", shader, "Emission Color")
    output = tree.nodes.new("ShaderNodeOutputMaterial")
    tree.links.new(shader.outputs["BSDF"], output.inputs["Surface"])
    glass.data.materials.append(material)
    group = bpy.data.objects.new("phone", None)
    bpy.context.collection.objects.link(group)
    body.parent = glass.parent = group
    group.rotation_euler = (math.radians(80), math.radians(0), math.radians(-24))
    aim = target()
    aim.location = Vector((0.0, 0.0, 0.42))
    place = Vector((0.7, -3.4, 1.1))
    camera(place, aim, 80.0, 6.0)
    normal = group.rotation_euler.to_matrix() @ Vector((0.0, 0.0, 1.0))
    reflector(mirrored(place, normal, aim, 5.0), 5.0, aim, 2.6)
    area("rim", Vector((2.8, 1.8, 1.2)), 1.6, 220.0, aim)
    area("key", Vector((-2.6, -2.4, 2.4)), 2.4, 160.0, aim)
    return 0.12


def main():
    arguments = options()
    bpy.ops.wm.read_factory_settings(use_empty=True)
    builders = {"paper": paper, "phone": phone, "case": etched}
    strength = builders[arguments.scene](arguments.assets)
    world(arguments.hdri, strength)
    scene = bpy.context.scene
    width, height = (int(v) for v in arguments.size.split("x"))
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = arguments.samples
    scene.cycles.use_denoising = True
    scene.cycles.denoiser = "OPENIMAGEDENOISE"
    scene.render.threads_mode = "FIXED"
    scene.render.threads = arguments.threads
    scene.render.resolution_x, scene.render.resolution_y = width, height
    scene.render.resolution_percentage = 100
    scene.view_settings.view_transform = "AgX"
    scene.render.filepath = str(arguments.out)
    bpy.ops.render.render(write_still=True)


main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
