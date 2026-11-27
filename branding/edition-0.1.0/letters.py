#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           branding/edition-0.1.0/letters.py
# Responsibility:   Draw the letters of the name "Danaco Nurt" on a construction grid.
# Dependencies:     argparse, math, sys, pathlib
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Draw the letters of the name "Danaco Nurt" on a construction grid.

The name has nine different letters: D, a, n, c, o, N, u, r and t. They are
built from straight stems and true circular arcs with one stroke width, the
way the mark is built. The ends of the strokes are cut by vertical lines, as
the ends of the arms of the chevrons are. The letter u is the letter n turned
by 180 degrees about the centre of its x-height, as the right chevron of the
mark is the left one turned by 180 degrees.

Units are the stroke widths of the construction: the x-height is X, the cap
height is C and the stroke is s. The functions return SVG path data with the
baseline at y = 0.

Usage:

  letters.py --out DIR [--board FILE]

The option --out writes wordmark-custom-*.svg; --board writes a review board.
"""

import argparse
import math
import sys
from pathlib import Path

X = 5.4          # x-height
C = 7.6          # cap height
OVERSHOOT = 0.19  # round letters rise above and sink below the lines
NAME = ("Danaco", "Nurt")

# Margins between letters, as fractions of the x-height, by side of a letter.
MARGIN = {"stem": 0.17, "round": 0.10, "open": 0.11}
WORD_SPACE = 0.95


def fmt(value):
    text = f"{value:.2f}".rstrip("0").rstrip(".")
    return "0" if text in ("-0", "") else text


class Pen:
    """Writes path data at an offset and scale; may turn the drawing by 180 degrees."""

    def __init__(self, ox, k, turn=None):
        self.ox, self.k, self.turn, self.parts = ox, k, turn, []

    def point(self, x, y):
        if self.turn is not None:
            x, y = self.turn[0] - x, self.turn[1] - y
        return f"{fmt((x + self.ox) * self.k)} {fmt(y * self.k)}"

    def M(self, x, y):
        self.parts.append("M" + self.point(x, y))

    def L(self, x, y):
        self.parts.append("L" + self.point(x, y))

    def A(self, rx, ry, large, sweep, x, y):
        self.parts.append(f"A{fmt(rx * self.k)} {fmt(ry * self.k)} 0 {large} {sweep} " + self.point(x, y))

    def Z(self):
        self.parts.append("Z")

    def rect(self, x0, y0, x1, y1):
        """A rectangle written clockwise on the screen; y0 is the upper edge."""
        self.M(x0, y0)
        self.L(x1, y0)
        self.L(x1, y1)
        self.L(x0, y1)
        self.Z()

    def data(self):
        return "".join(self.parts)


def ring(pen, cx, cy, rx, ry, s):
    pen.M(cx - rx, cy)
    pen.A(rx, ry, 1, 1, cx + rx, cy)
    pen.A(rx, ry, 1, 1, cx - rx, cy)
    pen.Z()
    ix, iy = rx - s, ry - s
    pen.M(cx - ix, cy)
    pen.A(ix, iy, 1, 0, cx + ix, cy)
    pen.A(ix, iy, 1, 0, cx - ix, cy)
    pen.Z()


def width_n():
    return 0.93 * X


def draw_o(pen, s):
    ring(pen, X / 2, -X / 2, X / 2, X / 2 + OVERSHOOT, s)


def draw_a(pen, s):
    ring(pen, X / 2, -X / 2, X / 2, X / 2 + OVERSHOOT, s)
    pen.rect(X - s, -X, X, 0)


def draw_c(pen, s):
    rx, ry = X / 2, X / 2 + OVERSHOOT
    cx, cy = rx, -X / 2
    ix, iy = rx - s, ry - s
    dx = min(0.55 * rx, 0.8 * ix)
    outer = ry * math.sqrt(1 - (dx / rx) ** 2)
    inner = iy * math.sqrt(1 - (dx / ix) ** 2)
    pen.M(cx + dx, cy - outer)
    pen.A(rx, ry, 1, 0, cx + dx, cy + outer)
    pen.L(cx + dx, cy + inner)
    pen.A(ix, iy, 1, 1, cx + dx, cy - inner)
    pen.Z()


def extent_c():
    rx = X / 2
    return 0.0, rx + min(0.55 * rx, 0.8 * (rx - 1.0))


def draw_n(pen, s):
    w = width_n()
    ro = w / 2
    ri = ro - s
    cy = -(X + OVERSHOOT) + ro
    pen.rect(0, -X, s, 0)
    pen.M(0, cy)
    pen.A(ro, ro, 0, 1, w, cy)
    pen.L(w - s, cy)
    pen.A(ri, ri, 0, 0, s, cy)
    pen.Z()
    pen.rect(w - s, cy, w, 0)


def draw_r(pen, s):
    ro = width_n() / 2
    ri = ro - s
    cy = -(X + OVERSHOOT) + ro
    dx = 0.72 * ri
    pen.rect(0, -X, s, 0)
    pen.M(0, cy)
    pen.A(ro, ro, 0, 1, ro + dx, cy - math.sqrt(ro * ro - dx * dx))
    pen.L(ro + dx, cy - math.sqrt(ri * ri - dx * dx))
    pen.A(ri, ri, 0, 0, s, cy)
    pen.Z()


def extent_r(s):
    ro = width_n() / 2
    return 0.0, ro + 0.72 * (ro - s)


def draw_D(pen, s):
    flat, rd = 1.5, C / 2
    ri = rd - s
    pen.M(0, -C)
    pen.L(flat, -C)
    pen.A(rd, rd, 0, 1, flat, 0)
    pen.L(0, 0)
    pen.Z()
    pen.M(s, -C + s)
    pen.L(s, -s)
    pen.L(flat, -s)
    pen.A(ri, ri, 0, 0, flat, -C + s)
    pen.Z()


def width_D():
    return 1.5 + C / 2


def width_N():
    return 6.0


def draw_N(pen, s):
    w = width_N()
    thick = 0.92 * s
    hw = thick
    for _ in range(30):
        theta = math.atan2(C, w - hw)
        hw = thick / math.sin(theta)
    pen.rect(0, -C, s, 0)
    pen.rect(w - s, -C, w, 0)
    pen.M(0, -C)
    pen.L(hw, -C)
    pen.L(w, 0)
    pen.L(w - hw, 0)
    pen.Z()


def draw_t(pen, s):
    top = 1.30 * X
    rt = 0.40 * X
    ri = rt - s
    cy = -rt
    dx = 0.85 * ri
    xe = rt + dx
    pen.M(xe, cy + math.sqrt(rt * rt - dx * dx))
    pen.A(rt, rt, 0, 1, 0, cy)
    pen.L(0, -top)
    pen.L(s, -top)
    pen.L(s, cy)
    pen.A(ri, ri, 0, 0, xe, cy + math.sqrt(ri * ri - dx * dx))
    pen.Z()
    pen.rect(-T_LEFT, -X, s + T_RIGHT, -X + s)


T_LEFT, T_RIGHT = 0.28 * X, 0.42 * X


def extent_t(s):
    rt = 0.40 * X
    ri = rt - s
    return -T_LEFT, max(rt + 0.85 * ri, s + T_RIGHT)


def glyph(name, s):
    """Return (draw function, minimum x, maximum x, left side, right side)."""
    if name == "o":
        return draw_o, 0.0, X, "round", "round"
    if name == "a":
        return draw_a, 0.0, X, "round", "stem"
    if name == "c":
        low, high = extent_c()
        return draw_c, low, high, "round", "open"
    if name == "n":
        return draw_n, 0.0, width_n(), "stem", "stem"
    if name == "u":
        return None, 0.0, width_n(), "stem", "stem"
    if name == "r":
        low, high = extent_r(s)
        return draw_r, low, high, "stem", "open"
    if name == "D":
        return draw_D, 0.0, width_D(), "stem", "round"
    if name == "N":
        return draw_N, 0.0, width_N(), "stem", "stem"
    if name == "t":
        low, high = extent_t(s)
        return draw_t, low, high, "open", "open"
    raise KeyError(name)


def draw_glyph(name, pen, s):
    if name == "u":
        draw_n(pen, s)
    else:
        glyph(name, s)[0](pen, s)


def word(text, s, k, start=0.0, previous=None):
    """Return the path data of the word, its end x and the right side of its last letter."""
    cursor = start
    parts = []
    for name in text:
        _, low, high, left, right = glyph(name, s)
        if previous is not None:
            cursor += (MARGIN[previous] + MARGIN[left]) * X
        offset = cursor - low
        pen = Pen(offset, k, turn=(width_n(), -X) if name == "u" else None)
        draw_glyph(name, pen, s)
        parts.append(pen.data())
        cursor = offset + high
        previous = right
    return "".join(parts), cursor, previous


def words(left_color, right_color, weights=(0.85, 1.35), cap_height=34.0):
    """Return the SVG paths of "Danaco Nurt" with the baseline at y = 0 and its width.

    The cap height is cap_height; the letters of the two words have the strokes
    weights[0] and weights[1] (units of the construction).
    """
    k = cap_height / C
    first, end, previous = word(NAME[0], weights[0], k)
    second, end2, _ = word(NAME[1], weights[1], k, start=end + WORD_SPACE * X, previous=None)
    svg = f'<path fill="{left_color}" d="{first}"/><path fill="{right_color}" d="{second}"/>'
    return svg, end2 * k


def main(argv):
    parser = argparse.ArgumentParser(description="Draw the letters of the name Danaco Nurt.")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--board", type=Path)
    arguments = parser.parse_args(argv)
    arguments.out.mkdir(parents=True, exist_ok=True)
    silver, white = "#C3C8CE", "#F6F7F8"
    text, width = words(silver, white)
    document = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -38 %s 46"><title>Danaco Nurt</title>%s</svg>\n'
        % (fmt(width + 4), text)
    )
    (arguments.out / "wordmark-custom-dark.svg").write_text(document, encoding="utf-8", newline="\n")
    if arguments.board is not None:
        arguments.board.write_text(board(), encoding="utf-8", newline="\n")
    print(f"{arguments.out} written")
    return 0


def board():
    rows = []
    specs = [
        ("two weights 0.85 / 1.35", (0.85, 1.35)),
        ("two weights 0.75 / 1.45", (0.75, 1.45)),
        ("one weight 1.10", (1.10, 1.10)),
        ("one weight 1.30", (1.30, 1.30)),
    ]
    height = 40 + 150 * len(specs)
    parts = [f'<rect width="1200" height="{height}" fill="#0B0C0E"/>']
    for index, (label, weights) in enumerate(specs):
        text, width = words("#C3C8CE", "#F6F7F8", weights, cap_height=76.0)
        y = 110 + index * 150
        parts.append(f'<g transform="translate(40 {y})">{text}</g>')
        parts.append(f'<text x="40" y="{y + 30}" fill="#7C8390" font-family="sans-serif" font-size="13">{label}</text>')
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="{height}" viewBox="0 0 1200 {height}">' + "".join(parts) + "</svg>\n"


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
