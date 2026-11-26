#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Construct the Danaco Nurt mark and write its SVG files.

The mark is two identical chevrons on a grid of 96 x 96 units. The left one
points left and is raised, the right one points right and is lowered; the
right chevron is the left one turned by 180 degrees about the centre of the
grid, so a step forward and its undo are one figure. The arms of both
chevrons meet at the angle THETA with the horizontal axis. The ends of the
arms are cut by vertical lines, the outer corner of a chevron is rounded with
the radius of half the stroke width, the inner corner is sharp. The gap
between the two parallel diagonal arms is the stream.

Usage:

  build.py [--out DIR] [--board FILE]

The option --board writes a review board with the variants, the application
icons and the mark at the sizes of the scale test.
"""

import argparse
import math
import sys
from pathlib import Path

GRID = 96.0
CENTRE = GRID / 2

# Colours of the identity.
GRAPHITE = "#0B0C0E"
GRAPHITE_SOFT = "#15171A"
SILVER = "#C3C8CE"
GOLD = "#D4AE6E"
GOLD_DEEP = "#92703E"
INK = "#2B2F36"
WHITE = "#F6F7F8"
NAVY = "#0A1224"
CYAN = "#4FD1F5"
BLUE = "#2F80ED"

# Construction of the standard mark and of the mark for 16 pixels, which has a
# heavier stroke and a wider stream.
STANDARD = {"theta": 36.5, "width": 11.0, "gap": 5.0, "apex_x": 20.0, "end_x": 45.5, "end_round": 1.3, "inner_round": 0.5}
SMALL = {"theta": 38.0, "width": 15.0, "gap": 8.0, "apex_x": 19.0, "end_x": 45.0, "end_round": 0.8, "inner_round": 0.0}


# Scale of the mark inside the tile of an application icon.
TILE_SCALE = 0.82
TILE_SCALE_SMALL = 0.86


def number(value):
    text = f"{value:.2f}".rstrip("0").rstrip(".")
    return "0" if text in ("-0", "") else text


def point(x, y):
    return f"{number(x)} {number(y)}"


def rounded_polygon(vertices, radii):
    """Return a closed path through the vertices with a fillet at each one."""
    count = len(vertices)
    corners = []
    for index, (x, y) in enumerate(vertices):
        px, py = vertices[index - 1]
        nx, ny = vertices[(index + 1) % count]
        a = (px - x, py - y)
        b = (nx - x, ny - y)
        la, lb = math.hypot(*a), math.hypot(*b)
        a, b = (a[0] / la, a[1] / la), (b[0] / lb, b[1] / lb)
        radius = radii[index]
        if radius <= 0:
            corners.append(((x, y), (x, y), 0.0, 1))
            continue
        angle = math.acos(max(-1.0, min(1.0, a[0] * b[0] + a[1] * b[1])))
        distance = radius / math.tan(angle / 2)
        first = (x + a[0] * distance, y + a[1] * distance)
        second = (x + b[0] * distance, y + b[1] * distance)
        travel_in = (x - px, y - py)
        travel_out = (nx - x, ny - y)
        cross = travel_in[0] * travel_out[1] - travel_in[1] * travel_out[0]
        corners.append((first, second, radius, 1 if cross > 0 else 0))
    parts = ["M" + point(*corners[0][1])]
    for first, second, radius, sweep in corners[1:] + corners[:1]:
        parts.append("L" + point(*first))
        if radius > 0:
            parts.append(f"A{number(radius)} {number(radius)} 0 0 {sweep} " + point(*second))
    parts.append("Z")
    return "".join(parts)


def left_chevron(spec, turn=False):
    """Return the path of the raised chevron that points left.

    With turn the path of the same chevron turned by 180 degrees about the
    centre of the grid is returned: the lowered chevron that points right.
    """
    theta = math.radians(spec["theta"])
    sin, cos = math.sin(theta), math.cos(theta)
    width, gap = spec["width"], spec["gap"]
    xa, xe = spec["apex_x"], spec["end_x"]
    half = width / 2
    # The distance between the lower arm of this chevron and the upper arm of
    # the turned one is width + gap; this fixes the height of the apex.
    ya = (GRID - ((GRID - 2 * xa) * sin - (width + gap)) / cos) / 2

    def edge_end(normal, direction):
        px, py = xa + half * normal[0], ya + half * normal[1]
        t = (xe - px) / direction[0]
        return xe, py + t * direction[1]

    up, down = (cos, -sin), (cos, sin)
    outer_up, outer_down = (-sin, -cos), (-sin, cos)
    inner_up, inner_down = (sin, cos), (sin, -cos)
    vertices = [
        edge_end(outer_up, up),
        (xa - half / sin, ya),
        edge_end(outer_down, down),
        edge_end(inner_down, down),
        (xa + half / sin, ya),
        edge_end(inner_up, up),
    ]
    if turn:
        vertices = [(GRID - x, GRID - y) for x, y in vertices]
    end_round, inner_round = spec["end_round"], spec["inner_round"]
    return rounded_polygon(vertices, [end_round, half, end_round, end_round, inner_round, end_round])


def mark(spec, left_fill, right_fill, defs=""):
    """Return the two paths of the mark as an SVG fragment."""
    return (
        f"{defs}"
        f'<path fill="{left_fill}" d="{left_chevron(spec)}"/>'
        f'<path fill="{right_fill}" d="{left_chevron(spec, True)}"/>'
    )


def svg(body, title, size=96, extra=""):
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" '
        f'viewBox="0 0 {number(size)} {number(size)}" role="img" aria-label="{title}"{extra}>'
        f"<title>{title}</title>{body}</svg>\n"
    )


def header():
    return (
        "<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->\n"
        "<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->\n"
        "<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->\n"
    )


def scaled(body, scale):
    """Scale a fragment about the centre of the grid."""
    c = number(CENTRE)
    return f'<g transform="translate({c} {c}) scale({number(scale)}) translate(-{c} -{c})">{body}</g>'


def metal_defs():
    return (
        "<defs>"
        '<linearGradient id="silver" x1="0" y1="0" x2="1" y2="1">'
        '<stop offset="0" stop-color="#E9ECEF"/><stop offset="1" stop-color="#8E949B"/></linearGradient>'
        '<linearGradient id="gold" x1="0" y1="0" x2="1" y2="1">'
        '<stop offset="0" stop-color="#F0D3A4"/><stop offset="1" stop-color="#BE9358"/></linearGradient>'
        "</defs>"
    )


def blue_defs():
    return (
        "<defs>"
        '<linearGradient id="stream" x1="0" y1="0" x2="1" y2="1">'
        f'<stop offset="0" stop-color="{CYAN}"/><stop offset="1" stop-color="{BLUE}"/></linearGradient>'
        "</defs>"
    )


def tile(background, body, radius=21):
    return f'<rect width="{number(GRID)}" height="{number(GRID)}" rx="{number(radius)}" fill="{background}"/>{body}'


def files():
    """Return the mapping from file name to content."""
    out = {}
    out["mark-dark.svg"] = svg(mark(STANDARD, SILVER, GOLD), "Danaco Nurt mark for dark backgrounds")
    out["mark-light.svg"] = svg(mark(STANDARD, INK, GOLD_DEEP), "Danaco Nurt mark for light backgrounds")
    out["mark-black.svg"] = svg(mark(STANDARD, GRAPHITE, GRAPHITE), "Danaco Nurt mark in black")
    out["mark-white.svg"] = svg(mark(STANDARD, WHITE, WHITE), "Danaco Nurt mark in white")
    out["mark-metal.svg"] = svg(
        mark(STANDARD, "url(#silver)", "url(#gold)", metal_defs()), "Danaco Nurt mark with metal gradients"
    )
    out["mark-blue.svg"] = svg(
        mark(STANDARD, "url(#stream)", "url(#stream)", blue_defs()), "Danaco Nurt mark in the blue of the product"
    )
    out["mark-16.svg"] = svg(mark(SMALL, SILVER, GOLD), "Danaco Nurt mark drawn for 16 pixels")
    small_body = scaled(mark(SMALL, SILVER, GOLD), TILE_SCALE_SMALL)
    out["app-icon-graphite.svg"] = svg(
        tile(GRAPHITE, scaled(mark(STANDARD, SILVER, GOLD), TILE_SCALE)), "Danaco Nurt application icon, graphite"
    )
    out["app-icon-light.svg"] = svg(
        tile(WHITE, scaled(mark(STANDARD, GRAPHITE, GRAPHITE), TILE_SCALE)), "Danaco Nurt application icon, light"
    )
    out["app-icon-blue.svg"] = svg(
        tile(NAVY, scaled(mark(STANDARD, "url(#stream)", "url(#stream)", blue_defs()), TILE_SCALE)),
        "Danaco Nurt application icon, blue",
    )
    out["app-icon-gold.svg"] = svg(
        tile(WHITE, scaled(mark(STANDARD, GOLD_DEEP, GOLD_DEEP), TILE_SCALE)), "Danaco Nurt application icon, gold on white"
    )
    out["app-icon-16.svg"] = svg(tile(GRAPHITE, small_body, 18), "Danaco Nurt application icon drawn for 16 pixels")
    out["favicon.svg"] = svg(
        mark(SMALL, "var(--left)", "var(--right)"),
        "Danaco Nurt",
        extra=' style="--left:#2B2F36;--right:#92703E"',
    ).replace(
        "<title>",
        "<style>@media (prefers-color-scheme: dark){svg{--left:#C3C8CE;--right:#D4AE6E}}</style><title>",
        1,
    )
    return out


def place(body, x, y, size):
    """Place a fragment of the grid with its upper left corner at x, y."""
    return f'<g transform="translate({number(x)} {number(y)}) scale({number(size / GRID)})">{body}</g>'


def board():
    """Return a review board as an SVG document of 1200 x 900 pixels."""
    dark = [
        (mark(STANDARD, SILVER, GOLD), 60),
        (mark(STANDARD, "url(#silver)", "url(#gold)", metal_defs()), 420),
        (mark(STANDARD, "url(#stream)", "url(#stream)", blue_defs()), 780),
    ]
    parts = [f'<rect width="1200" height="900" fill="{WHITE}"/>']
    parts.append(f'<rect width="1200" height="380" fill="{GRAPHITE}"/>')
    parts.append(f'<rect x="780" y="0" width="420" height="380" fill="{NAVY}"/>')
    for body, x in dark:
        parts.append(place(body, x, 30, 320))
    icons = [
        tile(GRAPHITE, scaled(mark(STANDARD, SILVER, GOLD), TILE_SCALE)),
        tile(WHITE, scaled(mark(STANDARD, GRAPHITE, GRAPHITE), TILE_SCALE)),
        tile(NAVY, scaled(mark(STANDARD, "url(#stream)", "url(#stream)", blue_defs()), TILE_SCALE)),
        tile(WHITE, scaled(mark(STANDARD, GOLD_DEEP, GOLD_DEEP), TILE_SCALE)),
        tile(GRAPHITE, scaled(mark(SMALL, SILVER, GOLD), TILE_SCALE_SMALL), 18),
    ]
    for index, icon in enumerate(icons):
        x = 60 + index * 190
        if index in (1, 3):
            parts.append(f'<rect x="{x - 6}" y="414" width="152" height="152" rx="26" fill="none" stroke="#D5D8DC"/>')
        parts.append(place(icon, x, 420, 140))
    # Scale test on dark and on light, drawn at the real pixel sizes.
    x = 60
    for size in (128, 64, 48, 32, 24, 16):
        spec = SMALL if size <= 24 else STANDARD
        parts.append(f'<rect x="{x - 8}" y="598" width="{size + 16}" height="{size + 16}" fill="{GRAPHITE}"/>')
        parts.append(place(mark(spec, SILVER, GOLD), x, 606, size))
        parts.append(f'<rect x="{x - 8}" y="{598 + 170}" width="{size + 16}" height="{size + 16}" fill="#FFFFFF" stroke="#E1E4E8"/>')
        parts.append(place(mark(spec, INK, GOLD_DEEP), x, 606 + 170, size))
        x += size + 40
    x = 760
    for body, fill in (
        (mark(STANDARD, GRAPHITE, GRAPHITE), "#FFFFFF"),
        (mark(STANDARD, WHITE, WHITE), GRAPHITE),
        (mark(STANDARD, "#808080", "#808080"), "#FFFFFF"),
    ):
        parts.append(f'<rect x="{x}" y="598" width="124" height="124" fill="{fill}" stroke="#E1E4E8"/>')
        parts.append(place(body, x + 14, 612, 96))
        x += 140
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="900" viewBox="0 0 1200 900">'
        + "".join(parts)
        + "</svg>\n"
    )


def main(argv):
    parser = argparse.ArgumentParser(description="Write the SVG files of the Danaco Nurt mark.")
    parser.add_argument("--out", type=Path, default=Path(__file__).resolve().parent / "svg")
    parser.add_argument("--board", type=Path, help="write a review board to this SVG file")
    arguments = parser.parse_args(argv)
    if arguments.board is not None:
        arguments.board.write_text(board(), encoding="utf-8")
        print(f"{arguments.board} written")
        return 0
    arguments.out.mkdir(parents=True, exist_ok=True)
    for name, content in files().items():
        (arguments.out / name).write_text(header() + content, encoding="utf-8")
        print(f"{arguments.out / name} written")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
