#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           branding/edition-0.1.0/export.py
# Responsibility:   Render the raster files of the identity from the SVG files.
# Dependencies:     argparse, struct, subprocess, sys, pathlib
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Render the raster files of the identity from the SVG files.

The program rsvg-convert renders the SVG files. The files written are the
Windows icon nurt.ico with the sizes 16 to 256 pixels, the browser icons
favicon.ico, favicon-32.png and apple-touch-icon.png, the application icon in
the sizes of the hicolor icon theme and the social card of 1200 x 630 pixels.
The sizes up to 32 pixels use the drawing for small sizes.

Usage:

  export.py [--svg DIR] [--out DIR]
"""

import argparse
import struct
import subprocess
import sys
from pathlib import Path

ICON_SIZES = (16, 24, 32, 48, 64, 128, 256)
HICOLOR_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
SMALL_LIMIT = 32
# The operating system of the touch icon rounds the corners itself and fills
# transparent pixels with black, so the rounded tile is rendered on its own
# colour: the corners stay opaque and the icon is a full square.
APPLE_TOUCH_BACKGROUND = "#0B0C0E"


def render(source, destination, width, height=None, background=None):
    command = ["rsvg-convert", "-w", str(width), "-h", str(height or width), str(source), "-o", str(destination)]
    if background:
        command[1:1] = ["-b", background]
    subprocess.run(command, check=True)


def icon_source(svg, size):
    return svg / ("app-icon-16.svg" if size <= SMALL_LIMIT else "app-icon-graphite.svg")


def social_card(lockup, destination):
    """Render the lockup centred on a card of 1200 x 630 pixels."""
    text = lockup.read_text(encoding="utf-8")
    view = text.split('viewBox="')[1].split('"')[0]
    inner = text[text.index(">", text.index("<svg")) + 1 : text.rindex("</svg>")]
    document = (
        '<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630">'
        '<rect width="1200" height="630" fill="#0B0C0E"/>'
        f'<svg x="330" y="50" width="540" height="530" viewBox="{view}">{inner}</svg></svg>'
    )
    source = destination.with_suffix(".svg")
    source.write_text(document, encoding="utf-8", newline="\n")
    try:
        render(source, destination, 1200, 630)
    finally:
        source.unlink()


def write_ico(path, images):
    """Write an ICO file whose entries are PNG images given as (size, bytes)."""
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries, data = b"", b""
    for size, content in images:
        entries += struct.pack(
            "<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(content), offset + len(data)
        )
        data += content
    path.write_bytes(header + entries + data)


def main(argv):
    parser = argparse.ArgumentParser(description="Render the raster files of the identity.")
    base = Path(__file__).resolve().parent
    parser.add_argument("--svg", type=Path, default=base / "svg")
    parser.add_argument("--out", type=Path, default=base / "raster")
    arguments = parser.parse_args(argv)
    svg, out = arguments.svg, arguments.out
    (out / "hicolor").mkdir(parents=True, exist_ok=True)
    pngs = {}
    for size in sorted(set(ICON_SIZES) | set(HICOLOR_SIZES)):
        target = out / "hicolor" / f"nurt-{size}.png"
        render(icon_source(svg, size), target, size)
        pngs[size] = target.read_bytes()
    write_ico(out / "nurt.ico", [(size, pngs[size]) for size in ICON_SIZES])
    write_ico(out / "favicon.ico", [(size, pngs[size]) for size in (16, 32, 48)])
    (out / "favicon-32.png").write_bytes(pngs[32])
    render(svg / "app-icon-graphite.svg", out / "apple-touch-icon.png", 180, background=APPLE_TOUCH_BACKGROUND)
    social_card(svg / "lockup-stacked-dark.svg", out / "social-card.png")
    for name in sorted(path.relative_to(out).as_posix() for path in out.rglob("*") if path.is_file()):
        print(out / name)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
