<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           branding/README.md -->
<!-- Responsibility:   Inventory of the branding folder and the rules of use of the Danaco Nurt identity. -->
<!-- Dependencies:     IDENTITY.md and the files of this folder. -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Danaco Nurt identity

This folder holds the production identity 1.0.0 of Danaco Nurt, accepted on 2026-11-26 and shipped with release 0.1.1: the mark, the wordmark, the lockups, the grounds, the application icons, the renders, the approval boards, the print specification and the reference drawings of the terminal banner. The assets are produced from the design pack of Danaco Official and are not regenerated in this repository. `IDENTITY.md` is the specification: decisions, values, construction, rules of use.

## The mark

Two identical chevrons on a grid of 96 x 96 units. The left chevron is silver and points back, the right chevron is gold and points forward; the right one is the left one turned by 180 degrees about the centre of the grid. A step and its undo are one figure. The channel between the two parallel arms is the stream (in Polish, *nurt*).

| Role | Value |
|---|---|
| Silver | `#DDE1E6` (221, 225, 230), 256-colour palette index 254 |
| Gold | `#C9A15A` (201, 161, 90), palette index 179 |
| Gold on light grounds | `#A3722A` |
| Ink on light grounds | `#2B2F36` |
| Graphite | `#0B0C0E` |
| Cyan, reserved for the pipe `\|>` | `#4FD1F5`, palette index 81 |

## Inventory

| Path | Content |
|---|---|
| `svg/` | mark, drawings for 16, 24 and 32 pixels, wordmark, lockups (horizontal, vertical, compact), grounds and stream tiles |
| `raster/` | PNG copies of the vector files, `social-card.png` and `social-card@2x.png` |
| `icons/macos/` | `nurt.icns` |
| `icons/windows/` | `nurt.ico` (embedded in `nurt.exe`), `nurt-tile-150.png`, `nurt-tile-44.png` |
| `icons/linux/hicolor/` | `NxN/apps/nurt.png` for 16, 22, 24, 32, 48, 64, 96, 128, 192, 256 and 512, `scalable/apps/nurt.svg`, `symbolic/apps/nurt-symbolic.svg` |
| `icons/web/` | `favicon.ico`, `favicon.svg`, `apple-touch-icon.png`, `icon-192.png`, `icon-512.png`, `icon-maskable-512.png`, `site.webmanifest` |
| `render/` | 3D renders and `render-metadata.json` |
| `boards/` | the five approval boards |
| `print/` | print specification (`print-spec.pdf`), print colour values and print geometry values; Pantone numbers are candidates until the first print |
| `terminal/` | reference drawings of the start banner in half blocks and in ASCII, with ANSI colour samples |
| `tokens.css`, `identity.json` | colour tokens and decided values |
| `guidelines-0.1.1.pdf` | the identity guidelines of the production identity 1.0.0 as shipped in release 0.1.1: mark, name, colour, grounds, finishing, print, icons and terminal banner; written by `book_production.py` through `book.py --edition 0.1.1` |
| `guidelines-0.1.0.pdf` | the identity guidelines of release 0.1.0, the earlier book, written by `book.py --edition 0.1.0` |
| `book.py` | the writer of the guidelines of both editions |
| `book_production.py` | the generator of the edition 0.1.1, which reads the design pack |
| `edition-0.1.0/` | the identity of release 0.1.0 in its original layout: the scripts `build.py`, `letters.py`, `lockup.py`, `export.py` and `test_branding.py`, the files `svg/` and `raster/` they write, and `render/` with `nurt-hero.png` and the Blender script `two_materials.py`; `book.py --edition 0.1.0` draws from it |

## Rules of use

1. Use the supplied files; do not redraw or retype the mark or the name.
2. Dark ground: `color-dark` or `white`; light ground: `color-light` or `black`; foil: `gold`; large screen formats: `metal`.
3. Clear space: the cap height of the wordmark in lockups, a quarter of the box height around the mark alone.
4. Below 48 pixels use the drawings for 32, 24 and 16 pixels; never scale the main drawing below 48 pixels.
5. The silver chevron is always left and raised, the gold chevron right and lowered; the tones are never swapped.
6. Gold appears in tint only as the deep gold `#A3722A` on light grounds, never as text on `#F6F7F8`.
7. Cyan is for the pipe `|>`, not for the logo.
8. The tile of 150 pixels for Windows is used only with a dark ground declared in a manifest; without a manifest it is not shipped.
9. The files in `edition-0.1.0/` belong to the identity of release 0.1.0; they are kept as the record of that identity and to reproduce `guidelines-0.1.0.pdf`, and are not used for the product.

## Guidelines

```text
python3 -I branding/book.py --edition 0.1.1 --pack PACK --python PACK_PYTHON
python3 -I branding/book.py --edition 0.1.0 --fonts FONT_DIRECTORY --hero branding/edition-0.1.0/render/nurt-hero.png
```

`book.py` writes the guidelines of one edition per run and names the file after the edition, `guidelines-EDITION.pdf` in this directory, unless `--out` is given. The edition 0.1.1 is the current one and the default; it describes the production identity 1.0.0 as shipped in release 0.1.1 and is written by `book_production.py` from the design pack `PACK` (the directory `design/brand/production` of the design repository of Danaco Official, which holds `backgrounds/`, `icons/`, `lockups/`, `mockups/`, `render/` and `build/kit/`). `PACK_PYTHON` is the interpreter of the Python environment of the pack, with Pillow, fontTools, uharfbuzz, shapely and numpy; `rsvg-convert` must be on the path. Two runs on the same sources write the same bytes, so `guidelines-0.1.1.pdf` is reproduced exactly.

The edition 0.1.0 is the earlier book, drawn by `book.py` itself from the construction in `edition-0.1.0/build.py` and the lettering in `edition-0.1.0/lockup.py`; it needs the package fontTools, `rsvg-convert` and the font files named in `book.py`. By default the scripts in `edition-0.1.0/` write their SVG and PNG files inside `edition-0.1.0/` (`letters.py` only where `--out` points), not over the files of the production identity.

## Use of the name and the mark

The names Danaco and Danaco Nurt and the marks in this folder are trademarks of Danaco Holding Group Sp. z o.o. The Apache License, Version 2.0 that covers the code does not grant rights to the names and the marks; see `docs/site/compatibility.md` for the use of the name.

Binary files (PNG, ICO, ICNS, PDF) carry no header. The JSON, the web manifest and the reference drawings in `terminal/` are data files that must stay byte exact and carry no header.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
