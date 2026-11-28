#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Write the subset web fonts of the site into the assets directory.

The site sets its text in Geist and Geist Mono (SIL Open Font License 1.1), the
typefaces of the identity of Danaco Nurt. Each variable font is cut down to the
characters the site needs: Basic Latin, Latin-1, Latin Extended-A (all Polish
letters), general punctuation, arrows and a few symbols. The result is written
as WOFF2 with the weight axis kept.

Usage:

  build_fonts.py SOURCE_DIRECTORY OUTPUT_DIRECTORY

SOURCE_DIRECTORY holds "Geist[wght].ttf" and "GeistMono[wght].ttf", either
directly or in the subdirectories "geist" and "geistmono" of the Google Fonts
collection. The script needs the package fontTools with Brotli.
"""

import sys
from pathlib import Path

from fontTools import subset
from fontTools.ttLib import TTFont

RANGES = {
    "basic_latin": (0x0020, 0x007E),
    "latin_1_supplement": (0x00A0, 0x00FF),
    "latin_extended_a": (0x0100, 0x017F),
    "general_punctuation_part_1": (0x2010, 0x2027),
    "general_punctuation_part_2": (0x2030, 0x203A),
    "euro_sign": (0x20AC, 0x20AC),
    "arrows": (0x2190, 0x2193),
    "minus_sign": (0x2212, 0x2212),
    "check_mark": (0x2713, 0x2713),
    "multiplication_sign": (0x00D7, 0x00D7),
}

FONTS = {
    "geist": ("Geist[wght].ttf", "geist-variable.woff2"),
    "geistmono": ("GeistMono[wght].ttf", "geist-mono-variable.woff2"),
}


def unicodes():
    for first, last in RANGES.values():
        yield from range(first, last + 1)


def cut(source: Path, target: Path) -> int:
    options = subset.Options()
    options.flavor = "woff2"
    options.layout_features = ["kern", "liga", "calt", "ccmp", "locl", "mark", "mkmk", "tnum", "zero", "ss01", "case"]
    options.notdef_outline = True
    options.name_IDs = [0, 1, 2, 3, 4, 5, 6, 13, 14]
    font = TTFont(str(source))
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=list(unicodes()))
    subsetter.subset(font)
    font.flavor = "woff2"
    font.save(str(target))
    return target.stat().st_size


def main(argv) -> int:
    if len(argv) != 3:
        print(__doc__)
        return 2
    sources, output = Path(argv[1]), Path(argv[2])
    output.mkdir(parents=True, exist_ok=True)
    for directory, (name, result) in FONTS.items():
        source = sources / directory / name if (sources / directory).is_dir() else sources / name
        size = cut(source, output / result)
        print(f"{output / result}: {size} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
