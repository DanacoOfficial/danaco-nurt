#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Write the identity guidelines of Danaco Nurt as a PDF of 1280 x 720 pages.

Every page is drawn from the construction in build.py and the lettering in
lockup.py; the text is converted to outlines, so the PDF does not depend on
installed fonts. The script needs the package fontTools, the program
rsvg-convert and the font files named below.

Usage:

  book.py --fonts DIR [--out FILE] [--hero PNG]

DIR holds InterDisplay-Regular.otf, InterDisplay-SemiBold.otf,
InterDisplay-Bold.otf, Inter-Regular.otf, Inter-SemiBold.otf,
JetBrainsMono-Regular.ttf and JetBrainsMono-Bold.ttf (all licensed under the
SIL Open Font License 1.1).
"""

import argparse
import importlib.util
import math
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
PDF_DATE_EPOCH = "1767225600"  # 2026-01-01T00:00:00Z, the date given to the PDF when none is set.


def load(name):
    spec = importlib.util.spec_from_file_location(f"nurt_{name}", HERE / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[f"nurt_{name}"] = module
    spec.loader.exec_module(module)
    return module


build = load("build")
lockup = load("lockup")
B = build

W, H = 1280, 720
MUTED = "#7C8390"


class Fonts:
    def __init__(self, directory):
        directory = Path(directory)
        names = {400: "Regular", 600: "SemiBold", 700: "Bold"}
        self.sora = {w: lockup.instance(directory / f"InterDisplay-{n}.otf", w) for w, n in names.items()}
        self.words_font = f"{directory / 'JetBrainsMono-Regular.ttf'}|{directory / 'JetBrainsMono-Bold.ttf'}"
        self.inter = lockup.instance(directory / "Inter-Regular.otf", 400)
        self.inter_bold = lockup.instance(directory / "Inter-SemiBold.otf", 600)
        self.mono = lockup.instance(directory / "JetBrainsMono-Regular.ttf", 400)
        self.mono_bold = lockup.instance(directory / "JetBrainsMono-Bold.ttf", 700)


def text(font, string, size, x, y, fill, tracking=0.0):
    """Return a path with the text converted to outlines; y is the baseline."""
    d, _ = lockup.outline(font, string, size, x, y, tracking=tracking)
    return f'<path fill="{fill}" d="{d}"/>'


def width_of(font, string, size, tracking=0.0):
    _, end = lockup.outline(font, string, size, 0.0, 0.0, tracking=tracking)
    return end


def page(body, dark=True, number=0, title="", fonts=None):
    background = B.GRAPHITE if dark else B.WHITE
    ink = B.SILVER if dark else B.INK
    parts = [f'<rect width="{W}" height="{H}" fill="{background}"/>', body]
    if fonts is not None and number:
        parts.append(text(fonts.inter, "DANACO NURT  /  IDENTITY GUIDELINES", 12, 64, 40, MUTED, tracking=1.6))
        parts.append(text(fonts.inter, title.upper(), 12, 64, H - 36, ink, tracking=1.6))
        parts.append(text(fonts.inter, f"{number:02d}", 12, W - 92, H - 36, MUTED, tracking=1.6))
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}">{"".join(parts)}</svg>\n'


def heading(f, string, dark=True, y=130):
    return text(f.sora[600], string, 40, 64, y, B.WHITE if dark else B.GRAPHITE_SOFT)


def paragraph(f, lines, x, y, dark=True, size=17, leading=28):
    ink = "#B7BDC6" if dark else "#3A3F47"
    return "".join(text(f.inter, line, size, x, y + i * leading, ink) for i, line in enumerate(lines))


def cover(f):
    stacked = lockup.files(HERE.parent / "branding" / "x", "caps") if False else None
    text_d, width = lockup.words("custom", B.SILVER, B.WHITE)
    scale = 1.9
    mark_size = 230
    body = B.place(B.mark(B.STANDARD, B.SILVER, B.GOLD), (W - mark_size) / 2, 90, mark_size)
    body += f'<g transform="translate({(W - width * scale) / 2:.1f} {90 + mark_size + 70}) scale({scale})">{text_d}</g>'
    body += text(f.inter, "IDENTITY GUIDELINES  /  VERSION 1.0  /  2026", 13, (W - 440) / 2, H - 90, MUTED, tracking=2.0)
    return body, None


def idea(f):
    mark = B.mark(B.STANDARD, B.SILVER, B.GOLD)
    body = heading(f, "One figure for a step and its undo")
    body += paragraph(f, [
        "The mark is two identical chevrons. The left one points back and is raised,",
        "the right one points forward and is lowered. Turn the mark by 180 degrees",
        "and you get the same figure: an operation and its undo are one shape.",
        "",
        "The silver chevron is the step back, the gold chevron is the step forward.",
        "The diagonal channel between the two parallel arms is the stream, the",
        "meaning of the name Nurt. The chevron is the sign shared with the family",
        "of Danaco marks.",
    ], 64, 200)
    body += B.place(mark, 700, 150, 300)
    body += f'<path d="M1010 300 h70" stroke="{MUTED}" stroke-width="2"/><path d="M1068 292 l12 8 l-12 8" fill="none" stroke="{MUTED}" stroke-width="2"/>'
    body += text(f.inter, "180", 13, 1020, 284, MUTED, tracking=1.0)
    rotated = f'<g transform="rotate(180 48 48)">{mark}</g>'
    body += B.place(rotated, 1090 - 20, 150, 300 * 0.62)
    return body, "Idea"


def construction(f):
    spec = B.STANDARD
    mark = B.mark(spec, B.SILVER, B.GOLD)
    scale = 5.2
    ox, oy = 64, 120
    grid = []
    for k in range(0, 97, 8):
        stroke = "#2A2E35"
        grid.append(f'<path d="M{k} 0 V96 M0 {k} H96" stroke="{stroke}" stroke-width="{0.12}" fill="none"/>')
    t = math.radians(spec["theta"])
    notes = []
    for x in (spec["end_x"], B.GRID - spec["end_x"]):
        notes.append(f'<path d="M{x} 0 V96" stroke="{B.GOLD}" stroke-width="0.25" stroke-dasharray="1.2 1.2" opacity="0.8"/>')
    notes.append(f'<circle cx="48" cy="48" r="1.2" fill="{B.WHITE}"/>')
    notes.append(f'<path d="M0 48 H96 M48 0 V96" stroke="{B.WHITE}" stroke-width="0.18" opacity="0.5"/>')
    body = heading(f, "Construction")
    body += f'<g transform="translate({ox + 520} {oy - 10}) scale({scale})"><rect width="96" height="96" fill="#101216"/>{"".join(grid)}{mark}{"".join(notes)}</g>'
    body += paragraph(f, [
        "Grid of 96 x 96 units.",
        "",
        f"Arms at {spec['theta']:g} degrees to the horizontal axis.",
        f"Stroke width {spec['width']:g} units.",
        f"Stream between the parallel arms {spec['gap']:g} units.",
        f"Arm ends cut by vertical lines at x = {spec['end_x']:g} and x = {B.GRID - spec['end_x']:g}.",
        "Outer corner rounded with half the stroke width,",
        "inner corner sharp, end corners 1.3 units.",
        "",
        "The right chevron is the left one turned by",
        "180 degrees about the centre of the grid.",
        "",
        "Every file is written by build.py from these values.",
    ], 64, 200)
    return body, "Construction"


def clear_space(f):
    spec = B.STANDARD
    mark = B.mark(spec, B.SILVER, B.GOLD)
    body = heading(f, "Clear space and minimum size")
    body += B.place(mark, 64, 170, 330)
    # Bounding box of the mark: x 14.5..81.5, y 13.3..82.7 on the grid (scale 330/96).
    k = 330 / 96
    x0, y0, x1, y1 = 64 + 14.5 * k, 170 + 13.3 * k, 64 + 81.5 * k, 170 + 82.7 * k
    pad = spec["width"] * k
    body += f'<rect x="{x0:.1f}" y="{y0:.1f}" width="{x1 - x0:.1f}" height="{y1 - y0:.1f}" fill="none" stroke="{MUTED}" stroke-width="1"/>'
    body += f'<rect x="{x0 - pad:.1f}" y="{y0 - pad:.1f}" width="{x1 - x0 + 2 * pad:.1f}" height="{y1 - y0 + 2 * pad:.1f}" fill="none" stroke="{B.GOLD}" stroke-width="1" stroke-dasharray="6 5"/>'
    body += paragraph(f, [
        "Keep a clear space equal to the stroke width of the mark",
        "(11 units) on every side of the mark.",
    ], 480, 220)
    body += paragraph(f, ["Minimum sizes", "", "24 px: the standard drawing.", "16 px: the drawing for small sizes (mark-16.svg,", "app-icon-16.svg)."], 480, 330)
    x = 480
    for size, spec_ in ((48, B.STANDARD), (32, B.STANDARD), (24, B.STANDARD), (16, B.SMALL)):
        body += f'<rect x="{x}" y="500" width="{size + 16}" height="{size + 16}" fill="#15171A"/>'
        body += B.place(B.mark(spec_, B.SILVER, B.GOLD), x + 8, 508, size)
        body += text(f.inter, f"{size} px", 12, x, 500 + size + 40, MUTED)
        x += size + 56
    return body, "Clear space and minimum size"


def colour(f):
    swatches = [
        ("Graphite", B.GRAPHITE, "Backgrounds, terminal"),
        ("Graphite soft", B.GRAPHITE_SOFT, "Panels"),
        ("Silver", B.SILVER, "Step back, text on dark"),
        ("Gold", B.GOLD, "Step forward, accents"),
        ("Gold deep", B.GOLD_DEEP, "Gold on light"),
        ("Ink", B.INK, "Mark on light"),
        ("White", B.WHITE, "Backgrounds, mono mark"),
        ("Navy", B.NAVY, "Product tile"),
        ("Cyan", B.CYAN, "Stream gradient start"),
        ("Blue", B.BLUE, "Stream gradient end"),
    ]
    body = heading(f, "Colour")
    for index, (name, value, use) in enumerate(swatches):
        column, row = index % 5, index // 5
        x, y = 64 + column * 236, 190 + row * 220
        rgb = tuple(int(value[i : i + 2], 16) for i in (1, 3, 5))
        body += f'<rect x="{x}" y="{y}" width="212" height="120" rx="6" fill="{value}" stroke="#30343B" stroke-width="1"/>'
        body += text(f.sora[600], name, 17, x, y + 150, B.WHITE)
        body += text(f.mono, value, 14, x, y + 174, "#B7BDC6")
        body += text(f.inter, f"RGB {rgb[0]} {rgb[1]} {rgb[2]}", 12, x, y + 192, MUTED)
        body += text(f.inter, use, 12, x, y + 208, MUTED)
    return body, "Colour"


def typography(f):
    L = lockup.load_letters()
    body = heading(f, "Typography")
    body += text(f.inter, "DRAWN LETTERS  /  THE NAME", 12, 64, 190, MUTED, tracking=1.6)
    name, width = L.words(B.SILVER, B.WHITE, cap_height=64.0)
    body += f'<g transform="translate(64 276)">{name}</g>'
    body += text(f.inter, "JETBRAINS MONO  /  CODE AND THE TERMINAL", 12, 64, 350, MUTED, tracking=1.6)
    body += text(f.mono, 'find "/srv/app" glob: "*.go" |> filter size > 1mb |> first 5', 20, 64, 390, B.GOLD)
    body += text(f.inter, "INTER DISPLAY AND INTER  /  HEADINGS, TEXT AND INTERFACE", 12, 64, 450, MUTED, tracking=1.6)
    body += text(f.sora[600], "Every change can be undone", 34, 64, 500, B.WHITE)
    body += text(f.inter, "Command shell and language where every change can be undone.", 22, 64, 548, "#D6DAE0")
    body += text(f.inter_bold, "Semibold for emphasis, regular for running text.", 17, 64, 584, "#B7BDC6")
    body += paragraph(f, ["The name is not typed in a font: its nine", "letters are drawn on the grid of the mark", "(next page). Code and the terminal use", "JetBrains Mono, headings and text use Inter.", "", "The three typefaces are licensed under the", "SIL Open Font License 1.1."], 900, 200, size=15, leading=24)
    return body, "Typography"


def lettering(f):
    L = lockup.load_letters()
    k = 14.3
    base = 340
    body = heading(f, "The letters")
    name, width = L.words(B.SILVER, B.WHITE, cap_height=L.C * k)
    for label, height in (("cap height", L.C), ("x-height", L.X), ("baseline", 0.0)):
        y = base - height * k
        body += f'<path d="M64 {y:.1f} H{64 + width + 8:.1f}" stroke="#30343B" stroke-width="1"/>'
        body += text(f.inter, label.upper(), 10, 64 + width + 20, y + 4, MUTED, tracking=1.2)
    body += f'<g transform="translate(64 {base})">{name}</g>'
    # n and u: the same letter turned by 180 degrees.
    k2 = 22.0
    n_path, n_units, _ = L.word("n", 1.1, k2)
    u_path, u_units, _ = L.word("u", 1.1, k2)
    n_end = n_units * k2
    y0 = 598
    nx = 64
    ux = nx + n_end + 170
    body += f'<g transform="translate({nx} {y0})"><path fill="{B.SILVER}" d="{n_path}"/></g>'
    body += f'<g transform="translate({ux:.1f} {y0})"><path fill="{B.GOLD}" d="{u_path}"/></g>'
    ax = nx + n_end + 40
    ay = y0 - L.X * k2 / 2
    body += f'<path d="M{ax:.1f} {ay:.1f} h66" stroke="{MUTED}" stroke-width="2"/><path d="M{ax + 56:.1f} {ay - 8:.1f} l12 8 l-12 8" fill="none" stroke="{MUTED}" stroke-width="2"/>'
    body += text(f.inter, "180", 13, ax + 22, ay - 14, MUTED, tracking=1.0)
    body += paragraph(f, [
        "Nine different letters: D, a, n, c, o, N, u, r, t.",
        "One stroke, true circular arcs, stems with flat ends.",
        "The ends of the strokes are cut by vertical lines,",
        "as the arms of the chevrons are.",
        "",
        "The letter u is the letter n turned by 180 degrees,",
        "as the right chevron is the left one turned by 180.",
        "Stroke: Danaco 0.85, Nurt 1.35 (x-height 5.4).",
    ], 560, 478, size=15, leading=22)
    return body, "The letters"


def variants(f):
    body = heading(f, "Variants")
    cells = [
        ("Dark", B.GRAPHITE, B.mark(B.STANDARD, B.SILVER, B.GOLD)),
        ("Light", "#FFFFFF", B.mark(B.STANDARD, B.INK, B.GOLD_DEEP)),
        ("Product blue", B.NAVY, B.mark(B.STANDARD, "url(#stream)", "url(#stream)", B.blue_defs())),
        ("Black", "#FFFFFF", B.mark(B.STANDARD, B.GRAPHITE, B.GRAPHITE)),
        ("White", B.GRAPHITE, B.mark(B.STANDARD, B.WHITE, B.WHITE)),
        ("Metal, large formats", B.GRAPHITE, B.mark(B.STANDARD, "url(#silver)", "url(#gold)", B.metal_defs())),
    ]
    for index, (name, background, mark) in enumerate(cells):
        x, y = 64 + (index % 3) * 396, 170 + (index // 3) * 250
        body += f'<rect x="{x}" y="{y}" width="372" height="210" rx="6" fill="{background}" stroke="#30343B" stroke-width="1"/>'
        body += B.place(mark, x + 118, y + 25, 136)
        body += text(f.inter, name.upper(), 11, x + 14, y + 200, MUTED if background != "#FFFFFF" else "#6B727C", tracking=1.4)
    return body, "Variants"


def icons(f):
    body = heading(f, "Application icons")
    tiles = [
        B.tile(B.GRAPHITE, B.scaled(B.mark(B.STANDARD, B.SILVER, B.GOLD), B.TILE_SCALE)),
        B.tile(B.WHITE, B.scaled(B.mark(B.STANDARD, B.GRAPHITE, B.GRAPHITE), B.TILE_SCALE)),
        B.tile(B.NAVY, B.scaled(B.mark(B.STANDARD, "url(#stream)", "url(#stream)", B.blue_defs()), B.TILE_SCALE)),
        B.tile(B.WHITE, B.scaled(B.mark(B.STANDARD, B.GOLD_DEEP, B.GOLD_DEEP), B.TILE_SCALE)),
    ]
    for index, tile in enumerate(tiles):
        x = 64 + index * 230
        if True:
            body += f'<rect x="{x - 4}" y="196" width="208" height="208" rx="38" fill="none" stroke="#30343B"/>'
        body += B.place(tile, x, 200, 200)
    x = 64
    for size in (128, 64, 48, 32, 24, 16):
        spec = B.SMALL if size <= 32 else B.STANDARD
        tile = B.tile(B.GRAPHITE, B.scaled(B.mark(spec, B.SILVER, B.GOLD), B.TILE_SCALE_SMALL if size <= 32 else B.TILE_SCALE), 18 if size <= 32 else 21)
        body += B.place(tile, x, 460, size)
        body += text(f.inter, f"{size}", 12, x, 460 + size + 24, MUTED)
        x += size + 36
    body += paragraph(f, ["The file nurt.ico holds the sizes 16 to 256 pixels.", "Sizes up to 32 pixels use app-icon-16.svg."], 880, 480, size=15, leading=24)
    return body, "Application icons"


def terminal(f):
    body = heading(f, "Terminal")
    body += f'<rect x="64" y="170" width="780" height="440" rx="8" fill="#0B0C0E" stroke="#30343B"/>'
    y = 222
    for row in build_banner_rows():
        left, right = row[:14], row[14:]
        body += text(f.mono, left, 22, 96, y, "#BCBCBC") + text(f.mono, right, 22, 96 + width_of(f.mono, left, 22), y, "#D7AF5F")
        y += 30
    body += text(f.mono, "Danaco Nurt 0.1.0", 15, 96, y + 28, "#D6DAE0")
    body += text(f.mono, "Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved.", 15, 96, y + 56, "#D6DAE0")
    body += text(f.mono, "Type 'help' to get started.", 15, 96, y + 84, "#D6DAE0")
    px = 96
    py = y + 140
    body += text(f.mono, "nurt", 22, px, py, B.GOLD) + text(f.mono, ">", 22, px + width_of(f.mono, "nurt", 22), py, B.SILVER)
    px += width_of(f.mono, "nurt> ", 22)
    for string, font, colour in (("find", f.mono_bold, B.GOLD), (' "/srv/app"', f.mono, B.CYAN), (" |> ", f.mono_bold, B.SILVER), ("first 5", f.mono, "#D6DAE0")):
        body += text(font, string, 22, px, py, colour)
        px += width_of(font, string, 22)
    body += f'<rect x="{px + 2:.1f}" y="{py + 2}" width="13" height="3" fill="#F6F7F8"/>'
    body += paragraph(f, [
        "The banner draws the name with ASCII characters:",
        "the first two letters in silver, the last two in gold,",
        "the colours of the two chevrons.",
        "",
        "The prompt: the name in gold, the indicator in silver,",
        "the cursor a blinking underscore. In a typed line the",
        "commands are gold, the pipe |> is silver and the",
        "strings are cyan.",
        "",
        "24-bit colour: silver #C3C8CE, gold #D4AE6E,",
        "cyan #4FD1F5. Other terminals: palette 250, 179, 81.",
        "NO_COLOR turns the colours off.",
    ], 880, 220, size=14, leading=22)
    return body, "Terminal"


def build_banner_rows():
    return [
        " _   _  _   _  ____   _____",
        r"| \ | || | | ||  _ \ |_   _|",
        r"|  \| || | | || |_) |  | |",
        r"| |\  || |_| ||  _ <   | |",
        r"|_| \_| \___/ |_| \_\  |_|",
    ]


def misuse(f):
    mark = B.mark(B.STANDARD, B.SILVER, B.GOLD)
    body = heading(f, "Do not")
    cases = [
        ("Stretch or squeeze", f'<g transform="translate(48 48) scale(1.45 0.8) translate(-48 -48)">{mark}</g>'),
        ("Recolour", B.mark(B.STANDARD, "#E5484D", "#3B82F6")),
        ("Rotate by another angle", f'<g transform="rotate(35 48 48)">{mark}</g>'),
        ("Mirror", f'<g transform="translate(96 0) scale(-1 1)">{mark}</g>'),
        ("Add effects", f'<g filter="url(#shadow)">{mark}</g>'),
        ("Outline", f'<g fill="none">{mark}</g>'.replace('fill="#C3C8CE"', 'fill="none" stroke="#C3C8CE" stroke-width="1"').replace('fill="#D4AE6E"', 'fill="none" stroke="#D4AE6E" stroke-width="1"')),
    ]
    defs = '<defs><filter id="shadow" x="-30%" y="-30%" width="160%" height="160%"><feDropShadow dx="3" dy="5" stdDeviation="3" flood-color="#000" flood-opacity="0.9"/></filter></defs>'
    body += defs
    for index, (name, drawing) in enumerate(cases):
        x, y = 64 + (index % 3) * 396, 170 + (index // 3) * 250
        body += f'<rect x="{x}" y="{y}" width="372" height="210" rx="6" fill="#15171A" stroke="#30343B"/>'
        body += B.place(drawing, x + 128, y + 22, 116)
        body += f'<path d="M{x + 340} {y + 20} l14 14 M{x + 354} {y + 20} l-14 14" stroke="#E5484D" stroke-width="2.5"/>'
        body += text(f.inter, name.upper(), 11, x + 14, y + 196, MUTED, tracking=1.4)
    return body, "Do not"


def material(f, image):
    import base64
    encoded = base64.b64encode(Path(image).read_bytes()).decode("ascii")
    body = f'<image href="data:image/png;base64,{encoded}" x="0" y="0" width="{W}" height="{H}" preserveAspectRatio="xMidYMid slice"/>'
    body += f'<rect x="0" y="0" width="{W}" height="120" fill="{B.GRAPHITE}" opacity="0.55"/>'
    body += text(f.sora[600], "Material study", 40, 64, 100, B.WHITE)
    body += text(f.inter, "The left chevron in silver, the right chevron in gold; rendered from the mark with Blender (Cycles).", 14, 64, H - 70, B.SILVER)
    return body, "Material study"


def label(f, string, x, y, fill=None):
    return text(f.inter, string.upper(), 11, x, y, fill or MUTED, tracking=1.4)


def card(x, y, w, h, fill, stroke="#30343B", rx=6):
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{fill}" stroke="{stroke}" stroke-width="1"/>'


def _channel(value):
    value = value / 255
    return value / 12.92 if value <= 0.03928 else ((value + 0.055) / 1.055) ** 2.4


def luminance(colour):
    red, green, blue = (int(colour[i : i + 2], 16) for i in (1, 3, 5))
    return 0.2126 * _channel(red) + 0.7152 * _channel(green) + 0.0722 * _channel(blue)


def contrast(first, second):
    high, low = sorted((luminance(first), luminance(second)), reverse=True)
    return (high + 0.05) / (low + 0.05)


def embed(document, x, y, width):
    """Place an SVG document of the lockups with its upper left corner at x, y."""
    view = document.split('viewBox="')[1].split('"')[0].split()
    ratio = float(view[3]) / float(view[2])
    inner = document[document.index(">", document.index("<svg")) + 1 : document.rindex("</svg>")]
    return (
        f'<svg x="{x:.1f}" y="{y:.1f}" width="{width:.1f}" height="{width * ratio:.1f}" '
        f'viewBox="{" ".join(view)}">{inner}</svg>'
    ), width * ratio


def contents(f, titles):
    body = heading(f, "Contents")
    entries = [(index + 1, title) for index, title in enumerate(titles) if title and title != "Contents"]
    half = (len(entries) + 1) // 2
    for index, (number, title) in enumerate(entries):
        column, row = divmod(index, half)
        x, y = 64 + column * 560, 210 + row * 34
        body += text(f.mono, f"{number:02d}", 15, x, y, B.GOLD)
        body += text(f.inter, title, 17, x + 56, y, "#D6DAE0")
    return body, "Contents"


def intro(f):
    body = heading(f, "Danaco Nurt")
    body += B.place(B.mark(B.STANDARD, B.SILVER, B.GOLD), 1090, 70, 110)
    body += text(f.inter, "Command shell and language where every change can be undone.", 22, 64, 190, "#D6DAE0")
    columns = [
        ("A shell and a language", ["One program for Linux, Windows and", "macOS. Commands are joined with the", "pipe |> and return structured results."]),
        ("Every change can be undone", ["The undo journal records changes to files,", "provider values and repository state; a", "change can be reviewed, undone and redone."]),
        ("For people and AI agents", ["The same program, the same results and", "the same journal for a person at a", "terminal and for an agent in a script."]),
    ]
    for index, (title, lines) in enumerate(columns):
        x = 64 + index * 392
        body += f'<path d="M{x} 280 h350" stroke="#30343B" stroke-width="1"/>'
        body += text(f.mono, f"0{index + 1}", 14, x, 320, B.GOLD)
        body += text(f.sora[600], title, 22, x, 358, B.WHITE)
        body += paragraph(f, lines, x, 400, size=15, leading=24)
    body += paragraph(f, ["This book describes the identity of the product: the mark, the name, the colours, the type,", "the icons, the terminal, the documents and the voice. Every drawing in it is written by the scripts", "in the directory branding, so a change of the construction changes every file at once."], 64, 520, size=15, leading=24)
    return body, "The product"


def anatomy(f):
    spec = B.STANDARD
    theta = math.radians(spec["theta"])
    sin, cos, tan = math.sin(theta), math.cos(theta), math.tan(theta)
    xa, width, gap = spec["apex_x"], spec["width"], spec["gap"]
    ya = (B.GRID - ((B.GRID - 2 * xa) * sin - (width + gap)) / cos) / 2
    k, ox, oy = 5.0, 390, 150

    def point(x, y):
        return ox + x * k, oy + y * k

    body = heading(f, "Anatomy of the mark")
    body += f'<rect x="{ox}" y="{oy}" width="{96 * k:.0f}" height="{96 * k:.0f}" fill="none" stroke="#2A2E35" stroke-width="1" stroke-dasharray="5 5"/>'
    body += f'<g transform="translate({ox} {oy}) scale({k})">{B.mark(spec, B.SILVER, B.GOLD)}</g>'
    notes = [
        ("left", 215, point(xa + 15, ya - 15 * tan), "Step back", "The left chevron, silver, raised."),
        ("left", 345, point(xa - 9.2 + 2.5, ya), "Round corner", "Radius of half the stroke width."),
        ("left", 470, point(47.2, 49.5), "The stream", "The channel between the arms: nurt."),
        ("right", 215, point(spec["end_x"], ya - (spec["end_x"] - xa) * tan), "Vertical cut", "Every arm ends on a vertical line."),
        ("right", 400, point(70, 96 - ya + 4), "Turn by 180", "The right chevron is the left one turned."),
        ("right", 540, point(B.GRID - (xa + 15), B.GRID - (ya - 15 * tan)), "Step forward", "The right chevron, gold, lowered."),
    ]
    for side, y, (tx, ty), title, line in notes:
        x = 64 if side == "left" else 940
        end = x + 250 if side == "left" else x - 14
        body += f'<path d="M{end:.1f} {y - 5} L{tx:.1f} {ty:.1f}" stroke="{MUTED}" stroke-width="1"/><circle cx="{tx:.1f}" cy="{ty:.1f}" r="3" fill="{B.WHITE}"/>'
        body += text(f.sora[600], title, 17, x, y, B.WHITE)
        body += text(f.inter, line, 13, x, y + 22, "#B7BDC6")
    body += label(f, "Grid 96 x 96", ox, oy + 96 * k + 22)
    return body, "Anatomy of the mark"


def lockups(f):
    documents = lockup.files("custom")
    body = heading(f, "Lockups")
    placements = [
        ("lockup-horizontal-dark.svg", B.GRAPHITE, 64, 170, 560, 190, 400),
        ("lockup-horizontal-light.svg", "#FFFFFF", 656, 170, 560, 190, 400),
        ("lockup-stacked-dark.svg", B.GRAPHITE, 64, 390, 330, 250, 220),
        ("lockup-stacked-light.svg", "#FFFFFF", 410, 390, 330, 250, 220),
    ]
    for name, background, x, y, w, h, width in placements:
        body += card(x, y, w, h, background)
        height = embed(documents[name], 0, 0, width)[1]
        body += embed(documents[name], x + (w - width) / 2, y + (h - height) / 2, width)[0]
    body += paragraph(f, ["The horizontal lockup is the default.", "The stacked lockup is for square and", "narrow formats.", "", "The smallest width of a lockup is 160 px.", "The clear space equals the stroke width", "of the mark.", "", "Two lockups are not combined and the", "name is never set in another typeface.", "(The variant -mono exists for the terminal", "and for places that cannot show outlines.)"], 790, 420, size=14, leading=22)
    return body, "Lockups"


def backgrounds(f):
    body = heading(f, "Backgrounds")
    cells = [
        ("Graphite", B.GRAPHITE, B.mark(B.STANDARD, B.SILVER, B.GOLD), (B.SILVER, B.GOLD)),
        ("White", "#FFFFFF", B.mark(B.STANDARD, B.INK, B.GOLD_DEEP), (B.INK, B.GOLD_DEEP)),
        ("Silver", B.SILVER, B.mark(B.STANDARD, B.GRAPHITE, B.GRAPHITE), (B.GRAPHITE,)),
        ("Gold", B.GOLD, B.mark(B.STANDARD, B.GRAPHITE, B.GRAPHITE), (B.GRAPHITE,)),
        ("Navy", B.NAVY, B.mark(B.STANDARD, "url(#stream)", "url(#stream)", B.blue_defs()), (B.CYAN, B.BLUE)),
        ("Gradient or photo", "url(#field)", B.mark(B.STANDARD, B.WHITE, B.WHITE), None),
    ]
    body += '<defs><linearGradient id="field" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#0A1224"/><stop offset="1" stop-color="#2F80ED"/></linearGradient></defs>'
    for index, (name, background, drawing, colours) in enumerate(cells):
        x, y = 64 + (index % 3) * 396, 170 + (index // 3) * 245
        body += card(x, y, 372, 215, background)
        body += B.place(drawing, x + 124, y + 22, 124)
        ink = B.WHITE if background in (B.GRAPHITE, B.NAVY, "url(#field)") else B.GRAPHITE
        note = "White mark, never on a busy area" if colours is None else "Contrast " + " / ".join(f"{contrast(c, background):.1f}:1" for c in colours)
        body += text(f.inter, f"{name.upper()}  /  {note}", 11, x + 14, y + 203, ink, tracking=1.2)
    return body, "Backgrounds"


def colour_use(f):
    body = heading(f, "Colour in use")
    bar = [(B.GRAPHITE, 0.70, "Graphite 70"), (B.SILVER, 0.17, "Silver 17"), (B.GOLD, 0.09, "Gold 9"), (B.CYAN, 0.04, "Cyan 4")]
    x = 64
    for colour, share, name in bar:
        w = 1152 * share
        body += f'<rect x="{x:.1f}" y="170" width="{w:.1f}" height="54" fill="{colour}" stroke="#30343B"/>'
        body += label(f, name, x + 8, 246)
        x += w
    body += paragraph(f, ["Graphite carries the surface, silver the text, gold marks the step forward and cyan appears only in", "strings and in the blue variant of the mark."], 64, 290, size=15, leading=24)
    pairs = [
        ("Silver on graphite", B.SILVER, B.GRAPHITE), ("White on graphite", B.WHITE, B.GRAPHITE), ("Gold on graphite", B.GOLD, B.GRAPHITE),
        ("Cyan on graphite", B.CYAN, B.GRAPHITE), ("Ink on white", B.INK, "#FFFFFF"), ("Gold deep on white", B.GOLD_DEEP, "#FFFFFF"), ("Graphite on gold", B.GRAPHITE, B.GOLD),
    ]
    body += label(f, "Contrast of text colours (WCAG 2.1)", 64, 366)
    for index, (name, fg, bg) in enumerate(pairs):
        y = 392 + index * 34
        ratio = contrast(fg, bg)
        rating = "AAA" if ratio >= 7 else "AA" if ratio >= 4.5 else "large text only" if ratio >= 3 else "fails"
        body += f'<rect x="64" y="{y - 20}" width="44" height="26" rx="3" fill="{bg}" stroke="#30343B"/>'
        body += text(f.sora[600], "Aa", 15, 72, y, fg)
        body += text(f.inter, name, 15, 124, y, "#D6DAE0")
        body += text(f.mono, f"{ratio:4.1f} : 1", 15, 400, y, B.GOLD)
        body += text(f.inter, rating, 15, 540, y, "#B7BDC6")
    return body, "Colour in use"


def platforms(f):
    body = heading(f, "Icons on every platform")
    columns = [
        ("Windows", ["nurt.ico holds 16, 24, 32, 48,", "64, 128 and 256 px and is", "embedded in nurt.exe by the", "resource script of the build."]),
        ("Linux", ["hicolor sizes 16 to 512 px:", "nurt-16.png ... nurt-512.png.", "The package installs them", "next to the executable."]),
        ("macOS", ["The tile is the same drawing;", "nurt.icns is made from the", "512 px file on a macOS host.", "(Not built in this release.)"]),
        ("Web", ["favicon.svg follows the colour", "scheme of the browser;", "favicon.ico, apple-touch-icon.png", "and the card 1200 x 630 px."]),
    ]
    for index, (name, lines) in enumerate(columns):
        x = 64 + index * 292
        body += card(x, 170, 272, 470, "#0F1114")
        body += label(f, name, x + 18, 200)
        big = B.tile(B.GRAPHITE, B.scaled(B.mark(B.STANDARD, B.SILVER, B.GOLD), B.TILE_SCALE), 21 if name != "macOS" else 22)
        body += B.place(big, x + 18, 224, 120 if name != "Web" else 90)
        sizes = (64, 48, 32, 24, 16)
        px = x + 18
        for size in sizes:
            spec = B.SMALL if size <= 32 else B.STANDARD
            tile = B.tile(B.GRAPHITE, B.scaled(B.mark(spec, B.SILVER, B.GOLD), B.TILE_SCALE_SMALL if size <= 32 else B.TILE_SCALE), 18 if size <= 32 else 21)
            body += B.place(tile, px, 380, size)
            body += text(f.inter, str(size), 11, px, 380 + size + 20, MUTED)
            px += size + 14
        body += paragraph(f, lines, x + 18, 520, size=13, leading=21)
    return body, "Icons on every platform"


def web(f):
    documents = lockup.files("custom")
    body = heading(f, "Documentation site")
    body += card(64, 160, 1152, 480, "#0F1114", rx=10)
    body += f'<rect x="64" y="160" width="1152" height="44" rx="10" fill="#1A1D22"/><rect x="64" y="194" width="1152" height="10" fill="#1A1D22"/>'
    tab = B.tile(B.GRAPHITE, B.scaled(B.mark(B.SMALL, B.SILVER, B.GOLD), B.TILE_SCALE_SMALL), 18)
    body += card(82, 170, 220, 34, B.GRAPHITE, rx=8) + B.place(tab, 96, 179, 16) + text(f.inter, "Installation - Danaco Nurt", 13, 122, 192, "#D6DAE0")
    body += card(330, 172, 560, 24, "#0B0C0E", rx=12) + text(f.inter, "nurt.danaco-group.pl/installation", 12, 348, 189, MUTED)
    header, height = embed(documents["lockup-horizontal-dark.svg"], 96, 228, 200)
    body += header
    for index, item in enumerate(("Installation", "Language", "Commands", "Journal", "Security")):
        body += text(f.inter, item, 14, 640 + index * 110, 262, "#B7BDC6")
    body += f'<path d="M64 296 H1216" stroke="#262A31" stroke-width="1"/>'
    body += text(f.sora[600], "Every change can be undone", 40, 96, 380, B.WHITE)
    body += text(f.inter, "Command shell and language where every change can be undone.", 18, 96, 416, "#B7BDC6")
    body += card(96, 450, 640, 150, "#0B0C0E")
    body += text(f.mono, "curl -fsSL https://nurt.danaco-group.pl/install.sh | sh", 15, 118, 490, B.SILVER)
    body += text(f.mono, 'nurt', 15, 118, 540, B.GOLD) + text(f.mono, ">", 15, 118 + width_of(f.mono, "nurt", 15), 540, B.SILVER)
    px = 118 + width_of(f.mono, "nurt> ", 15)
    for string, font, colour in (("find", f.mono_bold, B.GOLD), (' "/srv/app"', f.mono, B.CYAN), (" |> ", f.mono_bold, B.SILVER), ("first 5", f.mono, "#D6DAE0")):
        body += text(font, string, 15, px, 540, colour)
        px += width_of(font, string, 15)
    body += paragraph(f, ["The documentation is a site", "that works offline: the folder", "doc in every archive."], 800, 480, size=14, leading=22)
    return body, "Documentation site"


def print_pages(f):
    documents = lockup.files("custom")
    body = heading(f, "Documents")
    # Business card, front and back, 85 x 55 mm at 4 px per mm.
    body += card(64, 170, 340, 220, B.GRAPHITE, rx=4)
    body += embed(documents["lockup-stacked-dark.svg"], 64 + 95, 170 + 28, 150)[0]
    body += card(424, 170, 340, 220, "#FFFFFF", rx=4)
    body += B.place(B.mark(B.SMALL, B.INK, B.GOLD_DEEP), 446, 190, 40)
    body += text(f.sora[600], "Name Surname", 19, 446, 276, B.GRAPHITE)
    body += text(f.inter, "Title", 13, 446, 298, "#5A616B")
    body += text(f.mono, "support@danaco-group.pl", 12, 446, 340, B.INK)
    body += text(f.inter, "Danaco Holding Group Sp. z o.o.", 11, 446, 362, "#5A616B")
    # Letterhead, A4 at 1.2 px per mm.
    body += card(810, 170, 252, 372, "#FFFFFF", rx=2)
    body += embed(documents["lockup-horizontal-light.svg"], 830, 188, 112)[0]
    for row in range(12):
        body += f'<rect x="830" y="{250 + row * 18}" width="{212 - (row % 4) * 24}" height="5" rx="2" fill="#D6DAE0"/>'
    body += f'<path d="M830 496 H1042" stroke="{B.GOLD_DEEP}" stroke-width="1.5"/>'
    body += text(f.inter, "Danaco Holding Group Sp. z o.o.", 7.5, 830, 514, "#5A616B") + text(f.mono, "support@danaco-group.pl", 7.5, 830, 526, "#5A616B")
    # Envelope DL, 220 x 110 mm at 2 px per mm.
    body += card(64, 430, 440, 210, "#FFFFFF", rx=2)
    body += embed(documents["lockup-horizontal-light.svg"], 84, 448, 120)[0]
    for row in range(4):
        body += f'<rect x="300" y="{540 + row * 18}" width="{150 - row * 14}" height="5" rx="2" fill="#B7BDC6"/>'
    body += paragraph(f, ["Business card 85 x 55 mm,", "letterhead A4, envelope DL.", "", "White paper; graphite and gold", "deep in print. Silver and cyan", "are screen colours and are not", "printed.", "", "Print files: the black and the", "light variants of the lockups."], 540, 450, size=14, leading=22)
    return body, "Documents"


def social(f):
    documents = lockup.files("custom")
    body = heading(f, "Social and repository")
    card_path = HERE / "raster" / "social-card.png"
    import base64
    encoded = base64.b64encode(card_path.read_bytes()).decode("ascii")
    body += f'<image href="data:image/png;base64,{encoded}" x="64" y="170" width="560" height="294" /><rect x="64" y="170" width="560" height="294" fill="none" stroke="#30343B"/>'
    body += label(f, "Card 1200 x 630 px", 64, 486)
    avatar = B.tile(B.GRAPHITE, B.scaled(B.mark(B.STANDARD, B.SILVER, B.GOLD), B.TILE_SCALE))
    body += card(664, 160, 352, 214, "#22262C", rx=8)
    body += B.place(avatar, 680, 176, 150)
    body += f'<clipPath id="circle"><circle cx="925" cy="251" r="75"/></clipPath><g clip-path="url(#circle)">{B.place(B.tile(B.GRAPHITE, B.scaled(B.mark(B.STANDARD, B.SILVER, B.GOLD), 0.95), 0), 850, 176, 150)}</g>'
    body += label(f, "Avatar, square and round", 680, 360)
    # README banner 1280 x 320 at half size.
    body += card(64, 520, 640, 120, B.GRAPHITE, rx=2)
    body += B.place(B.mark(B.STANDARD, B.SILVER, B.GOLD), 90, 534, 92)
    body += text(f.mono, "Danaco ", 22, 206, 586, B.SILVER) + text(f.mono_bold, "Nurt", 22, 206 + width_of(f.mono, "Danaco ", 22), 586, B.WHITE)
    body += text(f.inter, "Command shell and language where", 12, 206, 610, "#B7BDC6") + text(f.inter, "every change can be undone.", 12, 206, 626, "#B7BDC6")
    body += label(f, "Repository banner", 64, 660)
    body += paragraph(f, ["The short description of the product", "is the sentence of the tagline:", "Command shell and language where", "every change can be undone", "", "On GitHub it ends with:", "for people and AI agents."], 724, 520, size=14, leading=22)
    return body, "Social and repository"


def voice(f):
    body = heading(f, "Name and voice")
    left = [
        "Danaco Nurt in full in titles and at the first mention,",
        "Nurt afterwards. The program and its command are nurt,",
        "set in a code font.",
        "",
        "The capitals NURT exist only in the drawing of the banner.",
        "",
        "The tagline is one sentence and is quoted exactly:",
        "Command shell and language where every change can be undone",
    ]
    right = [
        "Short and exact. A verb for an action: undo, redo, review.",
        "No exclamation marks, no humour, no metaphors.",
        "Say what happened and what the reader can do next.",
        "",
        "Text of the product, as it is printed:",
        "Danaco Nurt 0.1.0",
        "Type 'help' to get started.",
    ]
    body += label(f, "The name", 64, 190) + paragraph(f, left, 64, 224, size=16, leading=27)
    body += label(f, "The voice", 660, 190) + paragraph(f, right, 660, 224, size=16, leading=27)
    body += card(64, 480, 556, 140, "#0B0C0E") + text(f.mono, "nurt", 16, 84, 520, B.GOLD) + text(f.mono, " undo", 16, 84 + width_of(f.mono, "nurt", 16), 520, B.SILVER)
    body += text(f.mono, "Undone: 3 changes in /srv/app", 16, 84, 552, "#D6DAE0")
    body += card(660, 480, 556, 140, "#0B0C0E") + text(f.inter, "Not: \"Oops! Something went wrong\"", 16, 680, 520, "#B7BDC6") + text(f.inter, "But: what failed, where, and what to do.", 16, 680, 552, "#D6DAE0")
    return body, "Name and voice"


def family(f):
    body = heading(f, "The family")
    body += B.place(B.mark(B.STANDARD, B.SILVER, B.GOLD), 800, 150, 300)
    body += paragraph(f, [
        "Danaco Nurt is a product of Danaco Holding Group Sp. z o.o.",
        "",
        "The chevron is the sign that the marks of the Danaco family",
        "share. The mark of the group is not used as the mark of the",
        "product, and the mark of the product is not used for the group.",
        "",
        "The endorsement is a line of text under or beside the lockup:",
    ], 64, 210, size=17, leading=29)
    body += card(64, 470, 640, 150, B.GRAPHITE)
    documents = lockup.files("custom")
    body += embed(documents["lockup-horizontal-dark.svg"], 100, 500, 300)[0]
    body += text(f.inter, "A Danaco Holding Group product", 14, 100, 590, "#B7BDC6")
    return body, "The family"


def files_page(f):
    body = heading(f, "Files and regeneration")
    rows = [
        ("svg/", "marks, application icons, lockups, wordmarks, favicon"),
        ("raster/", "nurt.ico, favicon.ico, hicolor PNG files, social card"),
        ("render/", "the render of the mark in silver and gold, and its Blender script"),
        ("guidelines.pdf", "this book"),
        ("build.py", "the construction of the mark and the application icons"),
        ("letters.py", "the drawn letters of the name"),
        ("lockup.py", "the lockups and the wordmarks"),
        ("export.py", "the raster files"),
        ("book.py", "this book"),
    ]
    for index, (name, line) in enumerate(rows):
        y = 200 + index * 32
        body += text(f.mono, name, 15, 64, y, B.GOLD) + text(f.inter, line, 15, 280, y, "#D6DAE0")
    body += card(64, 510, 1152, 120, "#0B0C0E")
    for index, line in enumerate(["python3 -I branding/build.py", "python3 -I branding/lockup.py --font custom", "python3 -I branding/export.py", "python3 -I branding/book.py --fonts FONTS --hero branding/render/nurt-hero.png"]):
        body += text(f.mono, line, 14, 84, 542 + index * 24, B.SILVER)
    return body, "Files and regeneration"


def closing(f):
    body = B.place(B.mark(B.STANDARD, B.SILVER, B.GOLD), (W - 150) / 2, 160, 150)
    body += text(f.inter, "DANACO NURT  /  IDENTITY GUIDELINES  /  VERSION 1.0", 13, (W - 470) / 2, 400, MUTED, tracking=2.0)
    body += paragraph(f, ["The names Danaco and Danaco Nurt and the marks in this book are trademarks of Danaco Holding Group", "Sp. z o.o. The Apache License, Version 2.0 that covers the code does not grant rights to the names", "and the marks. Questions about the use of the identity: support@danaco-group.pl."], 172, 470, size=15, leading=26)
    return body, None


def main(argv):
    parser = argparse.ArgumentParser(description="Write the identity guidelines as a PDF.")
    parser.add_argument("--fonts", required=True, type=Path)
    parser.add_argument("--out", type=Path, default=HERE / "guidelines.pdf")
    parser.add_argument("--hero", type=Path, help="PNG of the rendered mark, added as the last page")
    arguments = parser.parse_args(argv)
    f = Fonts(arguments.fonts)
    sequence = [cover, "contents", intro, idea, anatomy, construction, clear_space, variants, lockups, lettering, typography, colour, colour_use, backgrounds, icons, platforms, web, terminal, print_pages, social, voice, family, misuse]
    results = [None if step == "contents" else step(f) for step in sequence]
    if arguments.hero is not None:
        results.append(material(f, arguments.hero))
    results.append(files_page(f))
    results.append(closing(f))
    titles = ["Contents" if result is None else result[1] for result in results]
    results[1] = contents(f, titles)
    pages = [page(body, number=index + 1, title=title, fonts=f) if title else page(body) for index, (body, title) in enumerate(results)]
    with tempfile.TemporaryDirectory() as directory:
        files = []
        for index, document in enumerate(pages, 1):
            path = Path(directory) / f"page-{index:02d}.svg"
            path.write_text(document, encoding="utf-8", newline="\n")
            files.append(str(path))
        # The date of creation of the PDF is the time of the run unless it is fixed,
        # so that two runs on the same sources write the same bytes.
        environment = dict(os.environ)
        environment.setdefault("SOURCE_DATE_EPOCH", PDF_DATE_EPOCH)
        subprocess.run(
            ["rsvg-convert", "-f", "pdf", "-o", str(arguments.out), *files], check=True, env=environment
        )
    print(f"{arguments.out} written ({len(pages)} pages)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
