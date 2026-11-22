#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Write the wordmark and the lockups of Danaco Nurt as SVG files.

The name "Danaco Nurt" is set in an open font (SIL Open Font License 1.1):
"Danaco" in a light weight and "Nurt" in a bold one, in capitals with wide
tracking (style caps) or in mixed case (style mixed). The letters are converted
to outlines, so the files do not depend on an installed font. The script needs
the package fontTools and the font file; build.py does not need either.

Usage:

  lockup.py --font FONT [--style title|mixed|caps] [--out DIR]

FONT is one font file, or two static files joined with a vertical bar (the
light word and the bold word), for example
JetBrainsMono-Regular.ttf|JetBrainsMono-Bold.ttf.
"""

import argparse
import importlib.util
import sys
from pathlib import Path

from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont


def load_build():
    spec = importlib.util.spec_from_file_location("nurt_build", Path(__file__).resolve().parent / "build.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


BUILD = load_build()
CAP_HEIGHT = 34.0
TEXT_GAP = 22.0


# Typographic styles of the name: the weights of the two words, the case and
# the tracking in em.
STYLES = {
    "title": {"weights": (500, 650), "upper": False, "tracking": -0.012, "space": 0.26},
    "mixed": {"weights": (500, 700), "upper": False, "tracking": 0.01, "space": 0.28},
    "caps": {"weights": (400, 700), "upper": True, "tracking": 0.14, "space": 0.55},
}


def instance(path, weight):
    font = TTFont(str(path))
    if "fvar" not in font:
        return font
    axes = {axis.axisTag for axis in font["fvar"].axes}
    location = {"wght": weight} if "wght" in axes else {}
    return instantiateVariableFont(font, location, inplace=False) if location else font


def outline(font, text, size, x, baseline, tracking=0.0):
    """Return the path data of the text and its end position."""
    glyphs = font.getGlyphSet()
    cmap = font.getBestCmap()
    scale = size / font["head"].unitsPerEm
    pen = SVGPathPen(glyphs, ntos=lambda value: f"{value:.2f}".rstrip("0").rstrip("."))
    cursor = x
    for character in text:
        name = cmap[ord(character)]
        glyphs[name].draw(TransformPen(pen, (scale, 0, 0, -scale, cursor, baseline)))
        cursor += glyphs[name].width * scale + tracking
    return pen.getCommands(), cursor


def words(font_path, left_color, right_color, style="mixed"):
    """Return the paths of "Danaco Nurt" and its width, with the first letter at x = 0."""
    spec = STYLES[style]
    if "|" in str(font_path):
        # Two static font files: the light word and the bold word.
        light_path, bold_path = str(font_path).split("|")
        light, bold = instance(light_path, 400), instance(bold_path, 700)
    else:
        light, bold = instance(font_path, spec["weights"][0]), instance(font_path, spec["weights"][1])
    size = CAP_HEIGHT / (bold["OS/2"].sCapHeight / bold["head"].unitsPerEm)
    first_text, second_text = ("DANACO", "NURT") if spec["upper"] else ("Danaco", "Nurt")
    tracking = size * spec["tracking"]
    first, cursor = outline(light, first_text, size, 0.0, 0.0, tracking=tracking)
    cursor += size * spec["space"] - tracking
    second, end = outline(bold, second_text, size, cursor, 0.0, tracking=tracking)
    end -= tracking
    return (
        f'<path fill="{left_color}" d="{first}"/><path fill="{right_color}" d="{second}"/>',
        end,
    )


def files(font_path, style="mixed"):
    build = BUILD
    out = {}
    themes = {
        "dark": (build.SILVER, build.GOLD, build.SILVER, build.WHITE),
        "light": (build.INK, build.GOLD_DEEP, "#5A616B", build.GRAPHITE_SOFT),
        "black": (build.GRAPHITE,) * 4,
        "white": (build.WHITE,) * 4,
    }
    for theme, (left, right, word_left, word_right) in themes.items():
        text, width = words(font_path, word_left, word_right, style)
        mark = build.mark(build.STANDARD, left, right)
        baseline = 48 + CAP_HEIGHT / 2
        text_x = 96 + TEXT_GAP - 14
        total = text_x + width + 6
        body = (
            build.place(mark, 0, 0, 96)
            + f'<g transform="translate({text_x:.2f} {baseline:.2f})">{text}</g>'
        )
        out[f"lockup-horizontal-{theme}.svg"] = (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {total:.2f} 96" role="img" '
            f'aria-label="Danaco Nurt"><title>Danaco Nurt</title>{body}</svg>\n'
        )
        # Stacked: the mark above the name, both centred; the mark is as wide
        # as 44 percent of the name.
        mark_size = 96 * 0.44 * width / 67.0
        stacked_width = width + 12
        name_x = (stacked_width - width) / 2
        mark_x = (stacked_width - mark_size) / 2
        name_y = mark_size + 18 + CAP_HEIGHT
        stacked = (
            build.place(mark, mark_x, 0, mark_size)
            + f'<g transform="translate({name_x:.2f} {name_y:.2f})">{text}</g>'
        )
        out[f"lockup-stacked-{theme}.svg"] = (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {stacked_width:.2f} {name_y + 14:.2f}" '
            f'role="img" aria-label="Danaco Nurt"><title>Danaco Nurt</title>{stacked}</svg>\n'
        )
        out[f"wordmark-{theme}.svg"] = (
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -{CAP_HEIGHT + 4:.2f} {width + 4:.2f} {CAP_HEIGHT + 12:.2f}" '
            f'role="img" aria-label="Danaco Nurt"><title>Danaco Nurt</title>{text}</svg>\n'
        )
    return out


def main(argv):
    parser = argparse.ArgumentParser(description="Write the wordmark and the lockups of Danaco Nurt.")
    parser.add_argument("--font", required=True, help="font file, or two files joined with |")
    parser.add_argument("--out", type=Path, default=Path(__file__).resolve().parent / "svg")
    parser.add_argument("--style", choices=sorted(STYLES), default="title")
    arguments = parser.parse_args(argv)
    arguments.out.mkdir(parents=True, exist_ok=True)
    for name, content in files(arguments.font, arguments.style).items():
        (arguments.out / name).write_text(BUILD.header() + content, encoding="utf-8")
        print(f"{arguments.out / name} written")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
