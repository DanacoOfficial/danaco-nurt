<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           branding.readme -->
<!-- Responsibility:   Inventory of the branding folder and the rules of use of the Danaco Nurt identity. -->
<!-- Dependencies:     IDENTITY.md and the files of this folder. -->
<!-- Author:           Danaco Official -->
<!-- Version:          production identity 1.0.0, release 0.1.1 -->
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

## Rules of use

1. Use the supplied files; do not redraw or retype the mark or the name.
2. Dark ground: `color-dark` or `white`; light ground: `color-light` or `black`; foil: `gold`; large screen formats: `metal`.
3. Clear space: the cap height of the wordmark in lockups, a quarter of the box height around the mark alone.
4. Below 48 pixels use the drawings for 32, 24 and 16 pixels; never scale the main drawing below 48 pixels.
5. The silver chevron is always left and raised, the gold chevron right and lowered; the tones are never swapped.
6. Gold appears in tint only as the deep gold `#A3722A` on light grounds, never as text on `#F6F7F8`.
7. Cyan is for the pipe `|>`, not for the logo.
8. The tile of 150 pixels for Windows is used only with a dark ground declared in a manifest; without a manifest it is not shipped.

## Use of the name and the mark

The names Danaco and Danaco Nurt and the marks in this folder are trademarks of Danaco Holding Group Sp. z o.o. The Apache License, Version 2.0 that covers the code does not grant rights to the names and the marks; see `docs/site/compatibility.md` for the use of the name.

Binary files (PNG, ICO, ICNS, PDF) carry no header. The JSON, the web manifest and the reference drawings in `terminal/` are data files that must stay byte exact and carry no header.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
