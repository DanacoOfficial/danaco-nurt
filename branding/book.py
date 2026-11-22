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
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent


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
    text_d, width = lockup.words(f.words_font, B.SILVER, B.WHITE, "title")
    scale = 1.9
    mark_size = 230
    body = B.place(B.mark(B.STANDARD, B.SILVER, B.GOLD), (W - mark_size) / 2, 90, mark_size)
    body += f'<g transform="translate({(W - width * scale) / 2:.1f} {90 + mark_size + 70}) scale({scale})">{text_d}</g>'
    body += text(f.inter, "IDENTITY GUIDELINES  /  VERSION 1.0  /  2026", 13, (W - 440) / 2, H - 90, MUTED, tracking=2.0)
    return page(body)


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
    return page(body, number=2, title="Idea", fonts=f)


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
    return page(body, number=3, title="Construction", fonts=f)


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
    return page(body, number=4, title="Clear space and minimum size", fonts=f)


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
    return page(body, number=5, title="Colour", fonts=f)


def typography(f):
    body = heading(f, "Typography")
    body += text(f.inter, "JETBRAINS MONO  /  THE NAME, CODE AND THE TERMINAL", 12, 64, 190, MUTED, tracking=1.6)
    first = width_of(f.mono, "Danaco ", 64)
    body += text(f.mono, "Danaco ", 64, 64, 270, B.SILVER, tracking=-0.8) + text(f.mono_bold, "Nurt", 64, 64 + first, 270, B.WHITE, tracking=-0.8)
    body += text(f.mono, 'find "/srv/app" glob: "*.go" |> filter size > 1mb |> first 5', 20, 64, 340, B.GOLD)
    body += text(f.inter, "INTER DISPLAY AND INTER  /  HEADINGS, TEXT AND INTERFACE", 12, 64, 430, MUTED, tracking=1.6)
    body += text(f.sora[600], "Every change can be undone", 34, 64, 480, B.WHITE)
    body += text(f.inter, "Command shell and language where every change can be undone.", 22, 64, 530, "#D6DAE0")
    body += text(f.inter_bold, "Semibold for emphasis, regular for running text.", 17, 64, 566, "#B7BDC6")
    body += paragraph(f, ["The name is set in JetBrains Mono:", "Danaco in the weight Regular and", "Nurt in the weight Bold. The same", "typeface sets the terminal, so the", "name looks like the program it", "names.", "", "All typefaces are licensed under", "the SIL Open Font License 1.1."], 900, 200, size=15, leading=24)
    return page(body, number=6, title="Typography", fonts=f)


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
    return page(body, number=7, title="Variants", fonts=f)


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
    return page(body, number=8, title="Application icons", fonts=f)


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
    return page(body, number=9, title="Terminal", fonts=f)


def build_banner_rows():
    sys.path.insert(0, "")
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
    return page(body, number=10, title="Do not", fonts=f)


def material(f, image):
    import base64
    encoded = base64.b64encode(Path(image).read_bytes()).decode("ascii")
    body = f'<image href="data:image/png;base64,{encoded}" x="0" y="0" width="{W}" height="{H}" preserveAspectRatio="xMidYMid slice"/>'
    body += f'<rect x="0" y="0" width="{W}" height="120" fill="{B.GRAPHITE}" opacity="0.55"/>'
    body += text(f.sora[600], "Material study", 40, 64, 100, B.WHITE)
    body += text(f.inter, "The left chevron in silver, the right chevron in gold; rendered from the mark with Blender (Cycles).", 14, 64, H - 70, B.SILVER)
    return page(body, number=11, title="Material study", fonts=f)


def main(argv):
    parser = argparse.ArgumentParser(description="Write the identity guidelines as a PDF.")
    parser.add_argument("--fonts", required=True, type=Path)
    parser.add_argument("--out", type=Path, default=HERE / "guidelines.pdf")
    parser.add_argument("--hero", type=Path, help="PNG of the rendered mark, added as the last page")
    arguments = parser.parse_args(argv)
    f = Fonts(arguments.fonts)
    pages = [cover(f), idea(f), construction(f), clear_space(f), colour(f), typography(f), variants(f), icons(f), terminal(f), misuse(f)]
    if arguments.hero is not None:
        pages.append(material(f, arguments.hero))
    with tempfile.TemporaryDirectory() as directory:
        files = []
        for index, document in enumerate(pages, 1):
            path = Path(directory) / f"page-{index:02d}.svg"
            path.write_text(document, encoding="utf-8")
            files.append(str(path))
        subprocess.run(["rsvg-convert", "-f", "pdf", "-o", str(arguments.out), *files], check=True)
    print(f"{arguments.out} written ({len(pages)} pages)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
