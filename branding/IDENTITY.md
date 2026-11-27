<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           branding/IDENTITY.md -->
<!-- Responsibility:   Specification of the production identity of Danaco Nurt: decisions, values, rules of use, inventory of the branding folder. -->
<!-- Dependencies:     the files of the branding folder. -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Danaco Nurt: production identity 1.0.0

Date 2026-11-26. The idea and the silhouette of the mark are unchanged: two chevrons, a silver `<` (a step back) and a gold `>` (a step forward), the second one being the first turned by 180 degrees about the centre of a 96 x 96 grid. This version corrects the optics of the mark, replaces the typeface of the name and draws the system around it: layouts, grounds, finishing, icons, print specification and applications. The assets are produced from the design pack of Danaco Official and are not regenerated in this repository; the numbers below are read from the same source. Studies, mockups, separation PDFs and build scripts stay in the design pack and are not part of this folder.

## 1. Decisions in one table

| Area | Before | Production 1.0.0 | Reason |
|---|---|---|---|
| Arms of the mark | 36.5 degrees | 36.87 degrees = slope 3 : 4 | A ratio of whole numbers: every point is set with a ruler and a compass. |
| Stroke : stream | 11 : 5 nominal, 5.9 clear (unequal along the band) | 11 : 5.5, equal along the whole band | The stream is half of the stroke and has one width everywhere; at 16 px the 5.9 clearance measured 1.34 px. |
| Cuts of the arms | x = 45.5, tips 5.0 units apart | x = 49.7, tips interlock by 3.4 units | No vertical line divides the figure; a plain pair `<>` is always divided by one. This is the detail that weakens the association with brackets. |
| Tones | the former silver and gold: 1.24 : 1 | silver #DDE1E6, gold #C9A15A: 1.83 : 1 | The pair keeps a lightness difference in grey (1.91 : 1 against 1.27 : 1). |
| Drawings for small sizes | one drawing for 16 to 32 px | three drawings: 16, 24 and 32 px, aligned to the pixel grid | Stream 2.0 px, stroke 3.0 to 4.0 px; the channel stays open at every size. |
| Typeface of the name | letters drawn on the mark grid (stock typeface in the mono variant: JetBrains Mono) | Geist 400 / 700 (SIL OFL), own spacing per pair, bevels on N, r, t | Compared with seven other typefaces on one sheet; Geist has flat cut terminals that take the cuts of the mark and a monospaced companion for the terminal. |
| Layouts | horizontal, stacked, mark | horizontal, vertical, compact, mark, wordmark | Each in nine colour variants, ten with the screen metal (the wordmark has no metal variant). |
| Grounds | graphite and light | seven grounds with a measured matrix of allowed pairings | Contrast measured, not assumed. |
| Finishing | one render with grey silver | flat metal, five 3D renders, foil and emboss, print values | See sections 7 and 8. |

## 2. Mark

### 2.1 Construction (sheet in the design pack, `boards/board-wordmark.png` section 05)

| Parameter | Value |
|---|---|
| Grid | 96 x 96 units, centre of the turn (48, 48) |
| Slope of the arms | 3 : 4 (36.8699 degrees) |
| Stroke | 11 units |
| Stream (clearance between the chevrons) | 5.5 units, equal along the band (measured 5.50) |
| Apex of the silver chevron | x = 20, centre line at y = 37.31 |
| Cuts of the arms | vertical, x = 49.7 (silver) and x = 46.3 (gold) |
| Radii | outer apex 5.5 (half of the stroke), four ends 1.2, inner apex 0.6 |
| Box of the mark | 67.0 x 77.28 units |
| Interlock of the tips | 3.4 units |
| Silver path (d, grid 96) | `identity.json`, key `mark.paths_main.left` |
| Gold path (d, grid 96) | `identity.json`, key `mark.paths_main.right` |

The gold chevron is the silver one rotated by 180 degrees about (48, 48); the design pack draws both from one list of vertices, so the identity of the turn is exact. Corners are circular arcs written as cubic curves.

### 2.2 Variants A, B, C and own micro-corrections (sheet in the design pack)

| Candidate | Change | Box | Stream at 16 px (box fitted) | Tips | Overlap with `<>` (IoU) | Verdict |
|---|---|---|---|---|---|---|
| Current | none | 67.0 x 70.4 | 1.34 px | apart 5.0 | 0.197 | reference |
| A  Wider stream | stream 5 to 8 | 67.0 x 66.7 | 1.90 px | apart 5.0 | 0.275 | rejected: the figure shortens to 66.7 and the stream reads as a gap, not as a flow; the overlap with the plain pair rises to 0.275 |
| B  Longer arms | cuts 45.5 to 48.5 | 67.0 x 74.8 | 1.07 px | interlock 1.0 | 0.190 | adopted in direction (cuts beyond the axis), taken further and combined with an equal stream |
| C  Narrower opening | angle 36.5 to 32 degrees | 67.0 x 58.9 | 1.45 px | apart 5.0 | 0.305 | rejected: the figure flattens to 58.9 and approaches the plain pair (0.305), the arms lose the 3 : 4 reading |
| Selected | 3 : 4, stream 5.5, cuts 49.7 | 67.0 x 77.3 | 1.14 px | interlock 3.4 | 0.199 | production drawing |

Findings stated plainly: the overlap with the plain pair is 0.19 to 0.20 for the current mark, variant B and the selected construction, and higher for A (0.275) and C (0.305), which therefore move toward the brackets. The overlap measure does not separate the selected figure from `<>` any better than the current one; what separates it is the interlock of the tips (no vertical line divides the two chevrons), which only the selected construction and, marginally, variant B have. The angle 3 : 4 and the equal stream are corrections of order and legibility, not of silhouette.

### 2.3 Optical compensation of gold against silver

Geometry stays identical for both chevrons (the turn is exact); the compensation is made in tone and in placement. (1) Tone: silver is the lighter, gold the deeper one; on the graphite ground the luminance-weighted centre of mass of the mark lies 4.3 units to the left and 3.4 units above the centre of the box (96 grid), on a light ground 1.4 and 1.1 units (computed from the area of each chevron times its luminance difference to the ground). (2) Placement: in the horizontal lockup the centre of the turn sits 3 cap units below the middle of the cap height, the value that holds on both grounds. (3) The wordmark takes the same logic in weights: Danaco steps back in regular, Nurt steps forward in bold.

### 2.4 Drawings for small sizes (files `svg/mark-<size>px-<variant>.svg`, `raster/mark-<size>px-<variant>.png`)

| Drawing | Used at | Stroke | Stream (clearance) | Cuts | Notes |
|---|---|---|---|---|---|
| S16 | 16 to 23 px | 3.00 px | 2.00 px | x = 42 and 54 | 6 units per pixel; faces of the cuts on pixel edges; no interlock (a fraction of a pixel would blur); square ends, outer apex rounded with half of the stroke, inner apex sharp |
| S24 | 24 to 31 px | 3.50 px | 2.11 px | x = 44 and 52 | 4 units per pixel; faces of the cuts on pixel edges; no interlock (a fraction of a pixel would blur); square ends, outer apex rounded with half of the stroke, inner apex sharp |
| S32 | 32 to 47 px | 4.00 px | 2.00 px | x = 48 and 48 | 3 units per pixel; faces of the cuts on pixel edges; no interlock (a fraction of a pixel would blur); square ends, outer apex rounded with half of the stroke, inner apex sharp |
| Main | from 48 px | 5.50 px at 48 px | 2.75 px at 48 px | x = 49.7 and 46.3 | rounded corners |

Lightness contrast silver : gold in grey scale at 16 to 96 px was measured on the delivered pixels of the icons: 1.87 to 1.89 : 1 (tokens: 1.91 : 1).

### 2.5 Clear space and minimum size

Mark alone: a quarter of the box height (19 units) on every side. Lockups: the cap height of the wordmark (x) on every side. Minimum size of the mark: 16 px with the drawing S16 (6 mm in print for ink and foil; embossing 19.3 mm, section 8.3).

## 3. Colour

Tokens (file `identity.json`, key `tokens`; names describe the function):

| Role | Token | HEX | sRGB | CMYK coated FOGRA39 | CMYK uncoated FOGRA47 | Pantone |
|---|---|---|---|---|---|---|
| Ground, main | `graphite` | #0B0C0E | 11 12 14 | 60 / 50 / 50 / 100 | 50 / 40 / 40 / 100 | Black 6 C (candidate) |
| Ground, soft | `graphite_soft` | #15171A | 21 23 26 | 60 / 50 / 50 / 100 | 50 / 40 / 40 / 100 | none (stock or tint) |
| Ground, deep black | `black_deep` | #050506 | 5 5 6 | 60 / 50 / 50 / 100 | 50 / 40 / 40 / 100 | none (stock or tint) |
| Step back (silver) | `silver` | #DDE1E6 | 221 225 230 | 16 / 9 / 8 / 0 | 13 / 7 / 6 / 1 | 877 C metallic; Cool Gray 2 C printed (candidates) |
| Step forward (gold) | `gold` | #C9A15A | 201 161 90 | 20 / 34 / 70 / 8 | 9 / 28 / 70 / 13 | 871 C metallic; 7562 C printed (candidates) |
| Ink on light | `ink` | #2B2F36 | 43 47 54 | 75 / 63 / 51 / 65 | 73 / 53 / 37 / 74 | 433 C (candidate, low certainty) |
| Gold on light | `gold_deep` | #A3722A | 163 114 42 | 28 / 50 / 89 / 22 | 14 / 46 / 99 / 24 | 872 C metallic (candidate, low certainty) |
| Ground, light | `paper` | #F6F7F8 | 246 247 248 | 4 / 2 / 3 / 0 | 3 / 2 / 2 / 0 | none (stock or tint) |
| Ground, warm light | `paper_warm` | #F2EEE6 | 242 238 230 | 6 / 6 / 11 / 0 | 5 / 5 / 8 / 1 | none (stock or tint) |
| Ground, gold | `gold_ground` | #D9BC85 | 217 188 133 | 16 / 25 / 53 / 3 | 9 / 19 / 48 / 8 | none (stock or tint) |

CMYK is computed with `transicc` (LittleCMS 2.17), relative colorimetric intent with black point compensation, profiles ISO Coated v2 (FOGRA39, TAC limit 300) and PSO Uncoated ISO12647 (FOGRA47, TAC limit 260); the commands, the colour difference (CIEDE2000) of every recipe and the rich-black study are in `print/print-colour-values.md`. Pantone entries are candidates to be confirmed against a swatch book at the first print; none is certain and none is quoted as a decision. Graphite prints as rich black: 60 / 50 / 50 / 100 (TAC 260) on coated, 50 / 40 / 40 / 100 (TAC 230) on uncoated.

Additional lettering tones: Danaco on dark `#C9CED5`, Nurt on dark `#F6F7F8`, Danaco on light `#4A515C`, Nurt on light `#0B0C0E`. Cyan `#4FD1F5` is reserved for the pipe `|>` in the terminal and in code samples.

Metal gradients (screen, from 128 px; `identity.json` does not carry them):

| Metal | Stops (offset: colour) |
|---|---|
| Silver, brushed | 0.0: #F7F9FB, 0.16: #C3C9D1, 0.36: #EEF1F4, 0.58: #9AA1AA, 0.8: #D8DCE2, 1.0: #8A919A |
| Gold, polished | 0.0: #F6E3B4, 0.22: #D6AE66, 0.46: #B9893F, 0.7: #F0D6A0, 1.0: #A9792F |
| Gold foil (single colour variant) | 0.0: #EBD29B, 0.22: #CFA45C, 0.45: #F2DBA6, 0.7: #B7863F, 1.0: #D9B672 |

Contrast of the tones (WCAG 2.1): silver on graphite 14.9 : 1; gold on graphite 8.1 : 1; ink on paper 12.5 : 1; gold deep on paper 3.9 : 1 (mark and lines only, not text); silver : gold 1.83 : 1; ink : gold deep 3.20 : 1.

## 4. Wordmark (board `boards/board-wordmark.png`)

### 4.1 Selection of the typeface

Eight typefaces, all SIL Open Font License 1.1, set at one cap height in 400 and 700 with the font kerning only (no tracking, no modification): JetBrains Mono (the previous mono wordmark), Inter, Instrument Sans, Manrope, Outfit, Sora, Plus Jakarta Sans, Geist. Criteria: Polish diacritics (all 18 tested letters present in every candidate), weight range (400 to 700 available in all), flat cut terminals that take the cuts of the mark, a companion for the terminal. Geist is chosen: a neutral geometric grotesque with flat, square-cut terminals on r and t and a plain N, so that a cut at the slope of the mark reads as a deliberate gesture and not as a defect of the letter, and the monospaced sibling Geist Mono for the terminal and code. Rejected for the name: Outfit and Sora (round geometric forms conflict with the cut ends of the mark), Manrope and Plus Jakarta Sans (soft terminals), Inter (wide default spacing, generic), JetBrains Mono (monospaced rhythm gives a code-editor look and loose word shape).

Source: Geist and Geist Mono, SIL Open Font License 1.1. The lettering in every file is converted to curves.

### 4.2 Setting

| Parameter | Value |
|---|---|
| Weights | Danaco 400, Nurt 700 |
| Tracking (per mille of the em) | Danaco +22, Nurt -4 |
| Pair corrections (per mille of the em, on top of the font kerning) | Da -10, an +4, na -4, ac +12, co -2, Nu +6, ur +6, rt +34 |
| Gap between the words | 40 cap units (cap height = 100) |
| Bevels on N, r, t | 100 per mille of the em, at 3 : 4 |
| Width of the wordmark | 823.1 cap units; x-height 74.6 |
| Closest approach of neighbouring letters (cap units) | Da 16.4, an 17.0, na 18.6, ac 18.1, co 13.3, Nu 17.4, ur 18.7, rt 10.7 |

Method of the pair corrections: for each pair the mean clearance between the ink of the two letters in the band of the x-height (depth limited to 0.2 em) was measured; the correction brings the clearances of each word to its mean (rounded to 1 per mille). The pair rt was then corrected by hand to +34 because the measure over the band hides the closest approach: the terminal of r and the crossbar of t came within 4.8 cap units; after the correction 10.7, the narrowest gap of the wordmark. Every pair was verified by exact polygon distance.

### 4.3 Letter modifications

N, r and t receive a cut at the slope of the arms of the mark (3 : 4): N on its top right and bottom left corners (the letter keeps its 180 degree symmetry, as the mark does), r on the terminal, t on the top of the stem. Counters, stems and curves stay as designed. Drawing: sheet section 03 of `boards/board-wordmark.png`.

### 4.4 Relation of the weights and alignment

Danaco regular (stem 11.8 cap units), Nurt bold (stem 21.1): ratio 1 : 1.79. The company name steps back, the product name steps forward. In the horizontal lockup the mark is 2.2 cap heights tall, the gap between the mark and the name 0.75 cap, the centre of the turn 3 cap units below the middle of the cap height. Compact lockup: mark 2.0, gap 0.68, the name Nurt alone. Vertical lockup: mark 3.8, gap 0.95, wordmark centred under the mark.

## 5. Layouts and variants (board `boards/board-variants.png`)

Folders: `svg/` (vector files) and `raster/` (PNG copies). File pattern: `lockup-<layout>-<variant>.svg|png` for the layouts horizontal, vertical and compact, `mark-<variant>.svg|png`, `wordmark-<variant>.svg|png`; small drawings `mark-<size>px-<variant>.svg|png`.

| Variant | Use | Mark | Lettering |
|---|---|---|---|
| `color-dark` | graphite, deep black, graphite with the trace | `#DDE1E6 / #C9A15A` | `#C9CED5 / #F6F7F8` |
| `color-light` | paper, warm paper, paper with the trace | `#2B2F36 / #A3722A` | `#4A515C / #0B0C0E` |
| `black` | paper, warm paper, gold ground; single-colour print | `#000000 / #000000` | `#000000 / #000000` |
| `white` | graphite, deep black; engraving, single-colour print | `#FFFFFF / #FFFFFF` | `#FFFFFF / #FFFFFF` |
| `grey` | grey print, photocopies | `#2F2F2F / #777777` | `#5C5C5C / #141414` |
| `grey-dark` | grey screens | `#E1E1E1 / #A4A4A4` | `#CDCDCD / #F4F4F4` |
| `gold` | deep black and graphite; foil stamping | foil gradient, 5 stops (section 3) | foil gradient, 5 stops (section 3) |
| `metal` | screens, from 128 px; no lettering gradient | silver and gold metal gradients (section 3) | `#C9CED5 / #F6F7F8` |
| `emboss-light` | blind emboss mockups, cotton paper | `#E9EBEE` | same as the plate |
| `emboss-dark` | blind emboss on dark card | `#16181C` | same as the plate |

Minimum sizes (width of the whole layout; the mark alone by height):

| Layout | Screen | Ink in print | Hot foil | Blind emboss |
|---|---|---|---|---|
| mark | 16 px (S16), 24 px (S24), 32 px (S32), 48 px main | 6 mm | 6.0 mm | 19.3 mm |
| horizontal | 96 px | 28 mm | 40.8 mm | 95.6 mm |
| vertical | 64 px | 20 mm | 30.8 mm | 51.7 mm |
| compact | 56 px | 20 mm | 20.2 mm | 52.0 mm |
| wordmark | 72 px | 22 mm | 30.8 mm | 51.7 mm |

The foil and emboss values were computed from the outlines (print specification, section 6 and 7 of `print/print-colour-values.md`) and replace the first estimates.

## 6. Grounds and pairings (files `svg/background-*.svg`, `raster/background-*.png`)

| Ground | HEX | File | Use |
|---|---|---|---|
| Graphite | #0B0C0E | `background-graphite.svg|png` (1920 x 1080 and 2560 x 1440) | main ground of the product: terminal, documentation, screens |
| Deep black | #050506 | `background-black-deep.svg|png` (1920 x 1080 and 2560 x 1440) | foil and metal in print, presentations |
| Light | #F6F7F8 | `background-paper.svg|png` (1920 x 1080 and 2560 x 1440) | documents, papers, light screens |
| Warm light | #F2EEE6 | `background-paper-warm.svg|png` (1920 x 1080 and 2560 x 1440) | stationery, packaging, emboss |
| Warm gold | #D9BC85 | `background-gold.svg|png` (1920 x 1080 and 2560 x 1440) | campaign grounds; only the black one-colour variant |
| Graphite with the trace of the stream | #0B0C0E | `background-stream-dark.svg|png` (1920 x 1080 and 2560 x 1440) | covers, social cards, headers of documentation |
| Light with the trace of the stream | #F6F7F8 | `background-stream-light.svg|png` (1920 x 1080 and 2560 x 1440) | light covers and headers |

The trace of the stream is a set of hairlines at the slope 3 : 4 fading out toward the centre, with one pair of lines, silver and gold, spaced by the stream of the mark (6 units of the grid at 4 px per unit plus 2 px of line); a tile that repeats without a seam for CSS and print is in `svg/pattern-stream-dark.svg` and `svg/pattern-stream-light.svg` (96 px, lines every 24 px along the normal).

Matrix of pairings, measured (a variant is allowed on a ground when the mark keeps 3 : 1 and the lettering 4.5 : 1 against the ground, WCAG 2.1; `metal` and `emboss-*` follow the material rule; every number is in the design pack):

| Variant | graphite | deep black | light | warm light | gold | graphite+trace | light+trace |
|---|---|---|---|---|---|---|---|
| `color-dark` | yes (8.1) | yes (8.5) | no (1.0) | no (1.1) | no (1.2) | yes (8.1) | no (1.0) |
| `color-light` | no (1.0) | no (1.0) | yes (3.9) | yes (3.6) | no (2.3) | no (1.0) | yes (3.9) |
| `black` | no (1.1) | no (1.0) | yes (19.6) | yes (18.1) | yes (11.5) | no (1.1) | yes (19.6) |
| `white` | yes (19.6) | yes (20.4) | no (1.1) | no (1.2) | no (1.8) | yes (19.6) | no (1.1) |
| `grey` | no (1.1) | no (1.1) | yes (4.2) | yes (3.9) | no (2.5) | no (1.1) | yes (4.2) |
| `grey-dark` | yes (7.8) | yes (8.2) | no (1.0) | no (1.1) | no (1.1) | yes (7.8) | no (1.0) |
| `gold` | yes (6.0) | yes (6.3) | no (3.0) | no (2.8) | no (1.8) | yes (6.0) | no (3.0) |
| `metal` | yes | yes | no | no | no | yes | no |
| `emboss-light` | no | no | yes | yes | no | no | no |
| `emboss-dark` | yes | yes | no | no | no | no | no |

Forbidden use (sheet section 05 of `boards/board-variants.png`): swapping the tones of the chevrons; stretching or squeezing; mirroring (the figure is turned, not reflected); outlining; shadows and glow; colour on a ground that keeps less than 3 : 1; busy or coloured backgrounds; rotation; another typeface for the name; changing the angle or the stroke; crowding the clear space; gradients that are not in the system.

## 7. Finishing for screens (board `boards/board-finishing.png`, sections 01 and 02)

Flat premium metal (`svg/mark-metal.svg`, `svg/lockup-<layout>-metal.svg`): silver brushed (6 stops, fine horizontal grain), gold polished (5 stops), a lit edge line of 0.55 units; for large formats and hero surfaces from 128 px; lettering stays flat; below 64 px and in the product the flat tones are used.

3D renders (folder `render/`, Blender Cycles, CPU): `render-hero-front.png`, `render-hero-angle.png`, `render-macro-edge.png`, `render-light-ground.png` (2560 x 1440), `render-hero-transparent.png` (2048 x 2048 RGBA), parameters in `render/render-metadata.json`.

Parameters read from `render/render-metadata.json` (Blender 5.2.1 LTS, Cycles, CPU, OpenImageDenoise, total 938 s of rendering):

| Render | Resolution | Samples | Time (s) | Lens (mm) | f-stop | Camera elevation (deg) | Colour transform |
|---|---|---|---|---|---|---|---|
| `render-hero-front.png` | 2560 x 1440 | 192 | 200 | 120.0 | 11.0 | 8.0 | Khronos PBR Neutral |
| `render-hero-angle.png` | 2560 x 1440 | 128 | 161 | 100.0 | 2.8 | 11.0 | Khronos PBR Neutral |
| `render-macro-edge.png` | 2560 x 1440 | 256 | 357 | 100.0 | 5.6 | 12.0 | Khronos PBR Neutral |
| `render-light-ground.png` | 2560 x 1440 | 192 | 172 | 120.0 | 11.0 | 5.0 | Khronos PBR Neutral |
| `render-hero-transparent.png` | 2048 x 2048 | 192 | 49 | 100.0 | 11.0 | 6.0 | Khronos PBR Neutral |

Materials of the front view: silver brushed, base #D6DAE0, metallic 1.0, roughness 0.32, anisotropy 0.7 along the horizontal axis, brush texture noise stretched 37 x along X, bump 0.10; gold champagne, base #C9A15A, metallic 1.0, roughness 0.2. World: photo_studio_01_2k.hdr (CC0, Poly Haven), strength 0.25.

Lights of the front view:

| Light | Kind | Position (mm) | Size (mm) | Emission |
|---|---|---|---|---|
| softbox_strip_left | emissive_card | [-118, -32, 46] | [26, 170] | 7.0 |
| softbox_strip_right | emissive_card | [118, -22, 44] | [26, 170] | 6.3 |
| softbox_top | emissive_card | [0, -45, 150] | [170, 95] | 3.0 |
| rim_back_left | area_light | [-78, 130, 78] | [16, 90] | - |
| rim_back_right | area_light | [78, 130, 78] | [16, 90] | - |
| reflector_front_silver | emissive_card | [-53.16, -120, 52] | [193.6764303759474, 150] | 2.7 |
| reflector_front_gold | emissive_card | [128.68, -120, 52] | [170, 150] | 2.0 |
| bevel_light_top_left | emissive_card | [-105, -20, 175] | [90, 34] | 6.0 |
| bevel_light_top_right | emissive_card | [105, -20, 175] | [90, 34] | 6.0 |
| bevel_light_floor_left | emissive_card | [-60, -15, 1.0] | [90, 40] | 3.5999999999999996 |
| bevel_light_floor_right | emissive_card | [60, -15, 1.0] | [90, 40] | 3.5999999999999996 |
| backdrop_wash | spot | [0, -350, 520] | - | - |
| floor_pool | emissive_card | [0, 60, 160] | [300, 200] | 0.9 |

Materials: brushed silver, metal 1.0, roughness 0.28 to 0.38, anisotropy 0.6 to 0.8 along the horizontal brushing; polished gold, metal 1.0, roughness 0.16 to 0.24; silver is brighter than gold in every view. Extrusion 8 units of the 96 grid, bevel 0.6 units in three segments. Light: strip softboxes on both sides and above, a rim light behind, a fill card in front of the mark, a studio HDRI (CC0, Poly Haven) for reflections only.

## 8. Print specification (folder `print/`)

Full document: `print/print-spec.pdf` (A4, drawn in the identity of Nurt). Values: `print/print-colour-values.md` and `.json`; geometry: `print/print-geometry-values.json`.

### 8.1 Separations

In the design pack (not in this repository), vector PDF 1.4, one page, trimmed to the logo with a margin of 3 mm, spot colours written as Separation: `print-foil-gold-<layout>.pdf` (FOIL_GOLD 100 %), `print-foil-silver-<layout>.pdf` (FOIL_SILVER), `print-foil-duo-gold-<layout>.pdf` and `print-foil-duo-silver-<layout>.pdf` (the mark in two foils, the name in one), `print-emboss-die-<layout>.pdf` (EMBOSS_DIE), `print-black-k100-<layout>.pdf` (K 100 %), `print-colour-cmyk-<layout>.pdf` (colour on a dark ground, CMYK recipes of section 3), for the layouts horizontal, vertical and mark. Previews simulated from the height map: `print-preview-*.png`.

### 8.2 Hot foil

Hot foil stamping with a heated die, high-gloss metallised foil on a polyester carrier; gold: warm yellow gold with a slight red component (screen reference #C9A15A); silver: bright neutral to slightly cool mirror silver. Line 0.25 mm and clear gap 0.4 mm at least; the gap rule decides the minimum widths (the pair rt of the wordmark: 10.7 cap units). No manufacturer or catalogue number is specified: the printer supplies swatches (at least three golds and two silvers) stamped on the real stock before the order is released; the chosen foil type and number are recorded in the brand book after the first print.

### 8.3 Blind emboss

Depth 0.3 to 0.5 mm, walls 45 to 55 degrees, line width 0.5 mm at least, inner corner radius 0.15 mm at least. The inner apex of the mark has the radius 0.6 units, which reaches 0.15 mm at a mark height of 19.3 mm: that is the smallest embossed mark. 12 mm fails (0.093 mm), 20 mm passes (0.155 mm), 30 mm passes (0.233 mm). Stocks: black card 400 to 600 g, cotton paper 300 to 600 g; no gloss stock.

### 8.4 File preparation

Curves only; spot colours named FOIL_GOLD, FOIL_SILVER, EMBOSS_DIE; overprint off for foil layers; registration marks supplied by the printer; bleed 3 mm; the checklist for the printer is in `print-spec.pdf`.

## 9. Applications (folder `icons/`; board `boards/board-applications.png`)

| Application | Files | Parameters |
|---|---|---|
| macOS | `icons/macos/nurt.icns` | 11 entries (icp4, icp5, icp6, ic07 to ic14); tile 824 on the 1024 canvas, squircle with exponent 5, shadow in two layers |
| Windows | `icons/windows/nurt.ico`, `nurt-tile-150.png`, `nurt-tile-44.png` | 16, 24, 32, 48, 64, 128, 256 px (PNG in ICO); 16, 24, 32 from the drawings S16, S24, S32 |
| Linux | `icons/linux/hicolor/<N>x<N>/apps/nurt.png`, `scalable/apps/nurt.svg`, `symbolic/apps/nurt-symbolic.svg` | N = 16, 22, 24, 32, 48, 64, 96, 128, 192, 256, 512; symbolic in `currentColor` |
| Web | `icons/web/favicon.ico`, `favicon.svg`, `apple-touch-icon.png`, `icon-192.png`, `icon-512.png`, `icon-maskable-512.png`, `site.webmanifest` | `favicon.svg` uses the drawing S16 without a tile, silver and gold on a dark theme, ink and deep gold on a light one (`prefers-color-scheme`) |
| Icon variants (design pack only) | `app-icon-graphite|light|gold.svg|png` | tile gradient #1A1D22 to #0B0C0E; the mark at 58 per cent of the tile; metal gradients from 128 px |
| Social card | `raster/social-card.png` (1200 x 630), `raster/social-card@2x.png` | variant `metal` of the horizontal lockup on the trace of the stream, the sentence of the product in Geist |
| Documentation (design pack only) | `mockup-docs-header-dark.png`, `-light.png` (1920 x 480), `mockup-docs-page.png` (1440 x 900) | mark 30 px high in the navigation, colour variant; cyan only in the pipe `|>` |
| Terminal | `terminal/terminal-banner.txt`, `terminal/terminal-banner-blocks.txt`, `terminal/terminal-banner-truecolor.ansi`, `terminal/terminal-banner-256.ansi` | the mark as 24 x 14 ASCII cells and 26 x 15 half-block cells, checked by test: turning the array by 180 degrees gives the same picture |
| Stationery (design pack only) | `mockup-business-card.png`, `mockup-letterhead.png`, `mockup-stationery-set.png` | front of the card: gold foil on black card; back and letterhead: blind emboss on paper; envelope: colour variant |

Terminal colours: silver 221;225;230 and gold 201;161;90 in true colour; in the 256 palette silver is index 254 (nearest entry, CIEDE2000 2.89), gold is index 179 (4.20, the nearest entry) and the pipe in cyan #4FD1F5 is index 81 (2.31). `terminal/terminal-banner-256.ansi` uses these indices.

## 10. Inventory of the branding folder

| Folder or file | Content |
|---|---|
| `svg/` | mark (10 variants), drawings 16, 24, 32 px (6 variants each), wordmark (9 variants), horizontal, vertical and compact lockups (10 variants each), 7 grounds, 2 stream tiles |
| `raster/` | PNG copies of the vector files, `social-card.png` and `social-card@2x.png` |
| `icons/` | macOS, Windows, Linux (hicolor, scalable, symbolic) and web icons |
| `render/` | 5 renders, `render-metadata.json` |
| `boards/` | `board-before-after.png`, `board-wordmark.png`, `board-variants.png`, `board-finishing.png`, `board-applications.png` (the five approval boards) |
| `print/` | print specification, print colour values and print geometry values |
| `terminal/` | reference drawings of the start banner |
| `tokens.css` | colour tokens |
| `identity.json` | decided values and the paths of the mark |

Every SVG starts with the full Danaco header as XML comments and ends with the footer; the lettering is outlines; no file depends on an external font or a script. The Windows tile of 150 px is used only with a dark ground declared in a manifest; without a manifest it is not shipped.

## 11. Rules of use (summary)

1. Use the supplied files; do not redraw or retype the mark or the name.
2. Dark ground: `color-dark` or `white`; light ground: `color-light` or `black`; foil: `gold`; screens with large formats: `metal`.
3. Clear space: x = cap height of the wordmark in lockups, a quarter of the box height around the mark alone.
4. Below 48 px use the drawings S32, S24, S16; never scale the main drawing down below 48 px.
5. The silver chevron is always left and raised, the gold one right and lowered; the tones are never swapped.
6. Gold appears in tint only as the deep gold (#A3722A) on light grounds, never as text on #F6F7F8.
7. Cyan is for the pipe `|>`, not for the logo.

## 12. Decisions of Danaco Official

1. The tones are silver `#DDE1E6` (221, 225, 230), gold `#C9A15A` (201, 161, 90), gold on light `#A3722A`, ink on light `#2B2F36` and graphite `#0B0C0E`. Cyan `#4FD1F5` is unchanged.
2. In the 256-colour palette silver is index 254 (nearest entry, CIEDE2000 2.89), gold is index 179 and cyan is index 81.
3. Pantone numbers stay candidates until the first print; they are quoted as candidates.
4. The Windows tile of 150 px is used only with a dark ground declared in a manifest; without a manifest it is not shipped.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
