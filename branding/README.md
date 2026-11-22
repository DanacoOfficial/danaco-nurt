<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Danaco Nurt identity

This directory holds the visual identity of Danaco Nurt: the mark, the name set in type, the application icons and the guidelines. The files are written by the scripts in this directory, so every drawing follows from one construction.

## The mark

Two identical chevrons on a grid of 96 x 96 units. The left chevron points back and is raised, the right chevron points forward and is lowered; the right one is the left one turned by 180 degrees about the centre of the grid. A step and its undo are one figure. The silver chevron is the step back, the gold chevron is the step forward, and the channel between the two parallel arms is the stream (in Polish, *nurt*).

| Value | Standard drawing | Drawing for 16 pixels |
|---|---|---|
| Angle of the arms | 36.5 degrees | 38 degrees |
| Stroke width | 11 | 15 |
| Stream between the parallel arms | 5 | 8 |
| Arm ends | cut by vertical lines | cut by vertical lines |
| Outer corner | radius of half the stroke width | radius of half the stroke width |

The clear space around the mark equals the stroke width. The smallest size is 24 pixels for the standard drawing and 16 pixels for the drawing for small sizes.

## Colour

| Name | Value | Use |
|---|---|---|
| Graphite | `#0B0C0E` | backgrounds, terminal |
| Graphite soft | `#15171A` | panels |
| Silver | `#C3C8CE` | step back, text on dark backgrounds |
| Gold | `#D4AE6E` | step forward, accents |
| Gold deep | `#B88E4F` | gold on light backgrounds |
| Ink | `#2B2F36` | mark on light backgrounds |
| White | `#F6F7F8` | light backgrounds, white mark |
| Navy | `#0A1224` | tile of the blue application icon |
| Cyan to blue | `#4FD1F5` to `#2F80ED` | gradient of the blue variant |

In the terminal the banner, the prompt and the highlighting of a typed line use these colours: commands gold, the pipe `|>` silver, strings cyan. A terminal that reports 24-bit colour shows the exact values; any other terminal gets the nearest colours of the 256-colour palette (silver 250, gold 179, cyan 81).

## Type

The name "Danaco Nurt" is set in JetBrains Mono, "Danaco" in the weight Regular and "Nurt" in the weight Bold. The same typeface sets code and the terminal, so the name looks like the program it names. Headings, text and interface use Inter Display and Inter. The fonts are licensed under the SIL Open Font License 1.1. The letters of the name in the files are outlines.

## Files

| Path | Content |
|---|---|
| `svg/mark-*.svg` | the mark for dark and light backgrounds, in black, in white, with metal gradients and in the blue of the product |
| `svg/mark-16.svg` | the drawing for 16 pixels |
| `svg/app-icon-*.svg` | application icons: graphite, light, blue, gold on white and the drawing for 16 pixels |
| `svg/favicon.svg` | icon of the documentation site; it follows the colour scheme of the browser |
| `svg/lockup-*.svg`, `svg/wordmark-*.svg` | the mark with the name, horizontal and stacked, and the name alone |
| `raster/nurt.ico` | icon of `nurt.exe` with the sizes 16 to 256 pixels |
| `raster/hicolor/` | application icon in the sizes of the hicolor icon theme |
| `raster/favicon.ico`, `raster/apple-touch-icon.png`, `raster/social-card.png` | icons of the web and the card of 1200 x 630 pixels |
| `render/` | render of the mark in silver and gold and the Blender script that makes it |
| `guidelines.pdf` | the identity guidelines |

## Regeneration

```text
python3 -I branding/build.py
python3 -I branding/lockup.py --font "JetBrainsMono-Regular.ttf|JetBrainsMono-Bold.ttf"
python3 -I branding/export.py
python3 -I branding/book.py --fonts FONT_DIRECTORY --hero branding/render/nurt-hero.png
```

`build.py` and `export.py` need only Python and `rsvg-convert`. `lockup.py` and `book.py` also need the package fontTools and the font files. The file `crates/nurt-cli/windows/nurt.ico` is a copy of `raster/nurt.ico`, and `docs/site/favicon.svg` is a copy of `svg/favicon.svg`.

## Use of the name and the mark

The names Danaco and Danaco Nurt and the marks in this directory are trademarks of Danaco Holding Group Sp. z o.o. The Apache License, Version 2.0 that covers the code does not grant rights to the names and the marks; see `docs/site/compatibility.md` for the use of the name.
