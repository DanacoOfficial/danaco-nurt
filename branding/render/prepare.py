#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           branding/render/prepare.py
# Responsibility:   Prepares the images that the mockup scenes of mockups.py need from the sources of the identity.
# Dependencies:     argparse, importlib.util, subprocess, sys, tempfile, pathlib, PIL
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Prepare the images that the mockup scenes of mockups.py need.

The files written to the output directory are

  mask-left.png, mask-right.png  the two chevrons of the mark, white on black
  bump.png                       both chevrons, softened, for the relief
  screen.png                     the screen of a phone with the application icon

The masks are drawn from the construction in ../build.py and rendered by
rsvg-convert. The package Pillow and the font files Inter SemiBold and Inter
Regular are needed.

Usage:

  prepare.py --out DIR --semibold FILE --regular FILE [--icon PNG] [--rsvg PROGRAM]
"""

import argparse
import importlib.util
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

HERE = Path(__file__).resolve().parent
SIZE = 2048
PAD = 24  # grid units around the 96 x 96 grid; the mark fills 47 percent of the image


def load_build():
    spec = importlib.util.spec_from_file_location("nurt_build", HERE.parent / "build.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def render_mask(rsvg, path_data, destination):
    extent = 96 + 2 * PAD
    document = (
        f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{-PAD} {-PAD} {extent} {extent}">'
        f'<rect x="{-PAD}" y="{-PAD}" width="{extent}" height="{extent}" fill="#000"/>'
        f'<path fill="#fff" d="{path_data}"/></svg>'
    )
    with tempfile.TemporaryDirectory() as directory:
        source = Path(directory) / "mask.svg"
        source.write_text(document, encoding="utf-8")
        subprocess.run([rsvg, "-w", str(SIZE), "-h", str(SIZE), str(source), "-o", str(destination)], check=True)
    Image.open(destination).convert("L").save(destination)


def screen(icon_path, semibold, regular, destination):
    width, height = 1170, 2532
    image = Image.new("RGB", (width, height), (11, 12, 14))
    pixels = image.load()
    for y in range(height):
        shade = y / height
        for x in range(0, width, 2):
            glow = max(0.0, 1.0 - (((x - width * 0.7) / width) ** 2 + ((y - height * 0.25) / height) ** 2) * 5.5)
            r = int(11 + 6 * shade + 10 * glow)
            g = int(12 + 8 * shade + 26 * glow)
            b = int(14 + 14 * shade + 60 * glow)
            pixels[x, y] = (r, g, b)
            if x + 1 < width:
                pixels[x + 1, y] = (r, g, b)
    draw = ImageDraw.Draw(image, "RGBA")
    draw.rounded_rectangle((width / 2 - 170, 52, width / 2 + 170, 52 + 100), radius=50, fill=(0, 0, 0, 255))
    draw.text((150, 110), "9:41", font=ImageFont.truetype(str(semibold), 52), fill=(246, 247, 248, 255), anchor="lm")
    # Placeholder icons of other applications, dim, in a grid of four columns.
    cell, left, top = 270, 100, 460
    for row in range(5):
        for column in range(4):
            if (row, column) == (2, 1):
                continue
            x, y = left + column * (cell + 20), top + row * (cell + 90)
            draw.rounded_rectangle((x, y, x + 210, y + 210), radius=47, fill=(34, 38, 46, 200))
    icon = Image.open(icon_path).convert("RGBA").resize((210 + 60, 210 + 60), Image.LANCZOS)
    ix, iy = left + 1 * (cell + 20) - 30, top + 2 * (cell + 90) - 30
    shadow = Image.new("RGBA", image.size, (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle((ix + 20, iy + 40, ix + 250, iy + 270), radius=60, fill=(0, 0, 0, 150))
    image.paste(Image.alpha_composite(image.convert("RGBA"), shadow.filter(ImageFilter.GaussianBlur(26))).convert("RGB"))
    image.paste(icon, (ix, iy), icon)
    draw = ImageDraw.Draw(image, "RGBA")
    draw.text((ix + 135, iy + 320), "Danaco Nurt", font=ImageFont.truetype(str(regular), 38), fill=(246, 247, 248, 255), anchor="mm")
    draw.rounded_rectangle((width / 2 - 200, height - 70, width / 2 + 200, height - 58), radius=6, fill=(246, 247, 248, 235))
    image.save(destination)


def main(argv):
    parser = argparse.ArgumentParser(description="Prepare the images of the mockup scenes.")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--semibold", required=True, type=Path)
    parser.add_argument("--regular", required=True, type=Path)
    parser.add_argument("--icon", type=Path, default=HERE.parent / "raster" / "hicolor" / "nurt-512.png")
    parser.add_argument("--rsvg", default="rsvg-convert")
    arguments = parser.parse_args(argv)
    arguments.out.mkdir(parents=True, exist_ok=True)
    build = load_build()
    spec = build.STANDARD
    render_mask(arguments.rsvg, build.left_chevron(spec), arguments.out / "mask-left.png")
    render_mask(arguments.rsvg, build.left_chevron(spec, True), arguments.out / "mask-right.png")
    both = ImageChops.lighter(Image.open(arguments.out / "mask-left.png"), Image.open(arguments.out / "mask-right.png"))
    both.filter(ImageFilter.GaussianBlur(SIZE / 700)).save(arguments.out / "bump.png")
    screen(arguments.icon, arguments.semibold, arguments.regular, arguments.out / "screen.png")
    print(f"{arguments.out} written")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
