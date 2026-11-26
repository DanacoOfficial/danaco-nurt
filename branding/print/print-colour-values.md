<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           branding.print.print-colour-values -->
<!-- Responsibility:   Print colour values of the Danaco Nurt brand colours (CMYK FOGRA39 and FOGRA47, TAC, CIEDE2000), rich black study, Pantone candidates, foil requirements and millimetre conversions. -->
<!-- Dependencies:     print-colour-values.json, print-geometry-values.json (same directory). -->
<!-- Author:           Danaco Official -->
<!-- Version:          production identity 1.0.0, release 0.1.1 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Danaco Nurt: print colour values and print geometry

Date 2026-11-26. Values marked certain are computed with the program transicc (LittleCMS 2.17) and from the outlines of the logo files. Section 5 (Pantone) and section 8 (foils) contain candidates and requirements that could not be verified on the server and are kept apart from the certain values.

## 1. Method (certain)

- Source: sRGB (transicc built-in profile `*sRGB`). Intent: relative colorimetric (`-t1`). Black point compensation on for sRGB to CMYK (`-b`).
- Coated paper: `/usr/share/color/icc/ISOcoated_v2_bas.ICC` (ISO Coated v2 (ECI), FOGRA39, coated paper), TAC limit 300 %.
- Uncoated paper: `/usr/share/color/icc/PSO_Uncoated_ISO12647_bas.ICC` (PSO Uncoated ISO12647 (ECI), FOGRA47, uncoated wood-free paper), TAC limit 260 %.
- LittleCMS ColorSpace conversion calculator 5.1 [LittleCMS 2.17].

Exact commands (R G B 0 to 255, CMYK in percent):

```sh
echo "R G B" | transicc -n -i'*sRGB' -o<profile> -t1 -b
echo "C M Y K" | transicc -n -i<profile> -o'*Lab' -t1
echo "C M Y K" | transicc -n -i<profile> -o'*sRGB' -t1 -b
echo "R G B" | transicc -n -i'*sRGB' -o'*Lab' -t1
# example, graphite #0B0C0E on coated paper before the rich black rule:
echo "11 12 14" | transicc -n -i'*sRGB' -o/usr/share/color/icc/ISOcoated_v2_bas.ICC -t1 -b
```

- Measure of reproduction: CIEDE2000 (kLCH = 1) in CIELAB D50 between the target colour (sRGB converted to Lab) and the Lab of the integer CMYK recipe read from the print profile (print appearance, no BPC on return). de2000_software_return uses the return conversion CMYK to sRGB with BPC instead.
- TAC correction: If the transicc recipe exceeds the TAC limit, an integer CMYK recipe within the limit with the lowest CIEDE2000 to the target is searched (seeded batched local search, 1 percent steps).
- Integer percent values are the recipes to be entered in the layout program; the float output of transicc is stored in the JSON file.

## 2. CMYK values, coated paper, FOGRA39 (certain)

| Colour | HEX | sRGB | CMYK C / M / Y / K (%) | TAC (%) | dE2000 return to sRGB | dE2000 print appearance | Recipe |
|---|---|---|---|---|---|---|---|
| Graphite | #0B0C0E | 11 12 14 | 60 / 50 / 50 / 100 | 260 | 2.25 | 5.69 | rich black (target L* below 10: darker than any print on this profile) |
| Graphite soft | #15171A | 21 23 26 | 60 / 50 / 50 / 100 | 260 | 5.12 | 4.17 | rich black (target L* below 10: darker than any print on this profile) |
| Black deep | #050506 | 5 5 6 | 60 / 50 / 50 / 100 | 260 | 1.19 | 6.64 | rich black (target L* below 10: darker than any print on this profile) |
| Silver | #DDE1E6 | 221 225 230 | 16 / 9 / 8 / 0 | 33 | 0.00 | 0.24 | transicc |
| Gold | #C9A15A | 201 161 90 | 20 / 34 / 70 / 8 | 132 | 0.41 | 0.73 | transicc |
| Ink | #2B2F36 | 43 47 54 | 75 / 63 / 51 / 65 | 254 | 0.00 | 3.72 | transicc |
| Gold deep | #A3722A | 163 114 42 | 28 / 50 / 89 / 22 | 189 | 0.00 | 1.89 | transicc |
| Paper | #F6F7F8 | 246 247 248 | 4 / 2 / 3 / 0 | 9 | 0.55 | 0.65 | transicc |
| Paper warm | #F2EEE6 | 242 238 230 | 6 / 6 / 11 / 0 | 23 | 0.00 | 0.16 | transicc |
| Gold ground | #D9BC85 | 217 188 133 | 16 / 25 / 53 / 3 | 97 | 0.23 | 0.36 | transicc |
| Wordmark Danaco, dark ground (supplementary) | #C9CED5 | 201 206 213 | 25 / 16 / 13 / 0 | 54 | 0.47 | 0.61 | transicc |
| Wordmark Danaco, light ground (supplementary) | #4A515C | 74 81 92 | 68 / 54 / 42 / 40 | 204 | 0.00 | 2.07 | transicc |
| Caption grey (supplementary) | #7C8390 | 124 131 144 | 53 / 40 / 30 / 15 | 138 | 0.83 | 0.92 | transicc |

## 3. CMYK values, uncoated paper, FOGRA47 (certain)

| Colour | HEX | sRGB | CMYK C / M / Y / K (%) | TAC (%) | dE2000 return to sRGB | dE2000 print appearance | Recipe |
|---|---|---|---|---|---|---|---|
| Graphite | #0B0C0E | 11 12 14 | 50 / 40 / 40 / 100 | 230 | 5.73 | 18.58 | rich black (target L* below 10: darker than any print on this profile) |
| Graphite soft | #15171A | 21 23 26 | 50 / 40 / 40 / 100 | 230 | 7.23 | 16.30 | rich black (target L* below 10: darker than any print on this profile) |
| Black deep | #050506 | 5 5 6 | 50 / 40 / 40 / 100 | 230 | 5.44 | 19.60 | rich black (target L* below 10: darker than any print on this profile) |
| Silver | #DDE1E6 | 221 225 230 | 13 / 7 / 6 / 1 | 27 | 0.78 | 0.89 | transicc |
| Gold | #C9A15A | 201 161 90 | 9 / 28 / 70 / 13 | 120 | 0.37 | 3.07 | transicc |
| Ink | #2B2F36 | 43 47 54 | 73 / 53 / 37 / 74 | 237 | 1.26 | 12.50 | transicc |
| Gold deep | #A3722A | 163 114 42 | 14 / 46 / 99 / 24 | 183 | 0.39 | 7.13 | transicc |
| Paper | #F6F7F8 | 246 247 248 | 3 / 2 / 2 / 0 | 7 | 0.71 | 0.57 | transicc |
| Paper warm | #F2EEE6 | 242 238 230 | 5 / 5 / 8 / 1 | 19 | 0.50 | 0.58 | transicc |
| Gold ground | #D9BC85 | 217 188 133 | 9 / 19 / 48 / 8 | 84 | 0.45 | 2.05 | transicc |
| Wordmark Danaco, dark ground (supplementary) | #C9CED5 | 201 206 213 | 20 / 12 / 9 / 3 | 44 | 0.24 | 1.04 | transicc |
| Wordmark Danaco, light ground (supplementary) | #4A515C | 74 81 92 | 64 / 45 / 29 / 50 | 188 | 0.62 | 8.69 | transicc |
| Caption grey (supplementary) | #7C8390 | 124 131 144 | 46 / 33 / 20 / 23 | 122 | 0.51 | 4.38 | transicc |

### Ink coverage (TAC) check

Raw transicc recipes above the limit (all are near-black colours; each is replaced by the rich black recipe of section 4):

- Graphite (fogra39): raw transicc recipe 83 / 74 / 63 / 88, TAC 308 % > 300 %
- Graphite (fogra47): raw transicc recipe 77 / 58 / 52 / 93, TAC 280 % > 260 %
- Graphite soft (fogra47): raw transicc recipe 78 / 55 / 48 / 89, TAC 270 % > 260 %
- Black deep (fogra39): raw transicc recipe 85 / 77 / 65 / 91, TAC 318 % > 300 %
- Black deep (fogra47): raw transicc recipe 77 / 59 / 54 / 94, TAC 284 % > 260 %

All delivered recipes are within the limits (300 % coated, 260 % uncoated): the highest TAC is 260 % on coated and 237 % on uncoated paper.

Reading: dE2000 up to 2 is within tolerance, 2 to 5 needs a proof, above 5 the colour is out of gamut. The dark colours (graphite, graphite soft, black deep, ink on uncoated paper) are darker than the darkest black the paper can print (L* 12 on coated, L* 30 on uncoated paper), so their dE2000 is high by physics, not by error; the recipe is the darkest neutral black that stays within the limit.

## 4. Rich black for graphite (certain)

### ISO Coated v2 (ECI), FOGRA39, coated paper, TAC limit 300 %

| CMYK (%) | TAC (%) | Within TAC limit | L* / a* / b* (print) | dE2000 to graphite #0B0C0E |
|---|---|---|---|---|
| 0 / 0 / 0 / 100 | 100 | yes | 17.4 / -0.0 / 0.6 | 9.0 |
| 30 / 20 / 20 / 100 | 170 | yes | 14.7 / -0.6 / 0.0 | 7.2 |
| 40 / 30 / 30 / 100 | 200 | yes | 13.7 / -0.4 / 0.2 | 6.5 |
| 50 / 40 / 40 / 100 | 230 | yes | 12.9 / -0.5 / 0.5 | 6.1 |
| 60 / 40 / 40 / 100 | 240 | yes | 12.5 / -1.6 / -0.4 | 6.1 |
| 60 / 50 / 50 / 100 | 260 | yes | 12.1 / -0.5 / 0.7 | 5.7 |
| 70 / 60 / 60 / 100 | 290 | yes | 11.4 / -0.6 / 1.0 | 5.3 |
| 75 / 68 / 67 / 90 | 300 | yes | 14.4 / -0.0 / 1.3 | 7.3 |

Chosen: **60 / 50 / 50 / 100** (TAC 260 %, L* 12.1, dE2000 to graphite 5.7). Rule: lowest TAC recipe of the ladder within the ink limit whose L* is at most 1.0 above the darkest ladder recipe within the limit; the unrestricted search result is listed for comparison only. Unrestricted search for comparison: 87 / 69 / 44 / 100 (TAC 300 %, L* 10.3).

### PSO Uncoated ISO12647 (ECI), FOGRA47, uncoated wood-free paper, TAC limit 260 %

| CMYK (%) | TAC (%) | Within TAC limit | L* / a* / b* (print) | dE2000 to graphite #0B0C0E |
|---|---|---|---|---|
| 0 / 0 / 0 / 100 | 100 | yes | 33.1 / 1.0 / 1.9 | 20.5 |
| 30 / 20 / 20 / 100 | 170 | yes | 32.0 / 0.7 / 1.3 | 19.6 |
| 40 / 30 / 30 / 100 | 200 | yes | 31.4 / 0.5 / 1.3 | 19.1 |
| 50 / 40 / 40 / 100 | 230 | yes | 30.8 / 0.4 / 1.1 | 18.6 |
| 60 / 40 / 40 / 100 | 240 | yes | 30.5 / 0.2 / 0.5 | 18.3 |
| 60 / 50 / 50 / 100 | 260 | yes | 30.1 / 0.2 / 0.9 | 18.0 |
| 70 / 60 / 60 / 100 | 290 | NO | 29.5 / -0.0 / 0.8 | 17.5 |
| 75 / 68 / 67 / 90 | 300 | NO | 30.7 / -0.0 / 1.1 | 18.5 |

Chosen: **50 / 40 / 40 / 100** (TAC 230 %, L* 30.8, dE2000 to graphite 18.6). Rule: lowest TAC recipe of the ladder within the ink limit whose L* is at most 1.0 above the darkest ladder recipe within the limit; the unrestricted search result is listed for comparison only. Unrestricted search for comparison: 74 / 62 / 24 / 100 (TAC 260 %, L* 29.4).

Justification: black alone (0 / 0 / 0 / 100) prints L* 17.4 on coated paper; the recipe 60 / 50 / 50 / 100 lowers it to L* 12.1 with TAC 260 %, neutral (a* -0.5, b* 0.7). Raising the CMY part further (70 / 60 / 60 / 100, TAC 290 %) gains only 0.7 L* for 30 points more ink and brings the sheet near the limit, so it is not used. On uncoated paper the range is small (L* 33.1 for K alone, 30.8 for 50 / 40 / 40 / 100), the recipe 60 / 50 / 50 / 100 (TAC 260 %) gains 0.7 L* more at 30 points more ink and sits exactly at the limit, so 50 / 40 / 40 / 100 is used. Thin lines and small text are always K 100 only.

## 5. Pantone: candidates only, not verified

**Every entry of this section is a "candidate to be confirmed against a swatch book".** No Pantone swatch book, no spectral data and no Pantone Lab table are available in the production environment. The candidates were chosen from memory by colour family (hue, lightness, metallic or solid) and are NOT measured or computed matches. No hex or Lab values of Pantone colours are given because they could not be verified. Each candidate must be confirmed against the physical Pantone Formula Guide (Coated and Uncoated) and the Pantone Metallics Guide under D50 light, and then recorded here with the edition of the guide.

| Brand colour | HEX | Candidate | Kind | Certainty (from memory) | Note |
|---|---|---|---|---|---|
| Graphite | #0B0C0E | PANTONE Black 6 C (kandydat do potwierdzenia wzornikiem) | solid ink, non-metallic | name: high; match: medium | Neutral-cool very dark ink; alternative to the rich black recipe on coated paper. |
| Graphite | #0B0C0E | PANTONE Neutral Black C (kandydat do potwierdzenia wzornikiem) | solid ink, non-metallic | name: high; match: medium | Neutral black; check the undertone against the cool graphite. |
| Graphite soft | #15171A | none | - | - | No candidate given: no certain Pantone name could be recalled for this colour. It is reproduced with the process recipe of this document, as a tint, or by the stock itself. |
| Black deep | #050506 | none | - | - | No candidate given: no certain Pantone name could be recalled for this colour. It is reproduced with the process recipe of this document, as a tint, or by the stock itself. |
| Silver | #DDE1E6 | PANTONE 877 C (kandydat do potwierdzenia wzornikiem) | metallic ink | name and type: high; match to #DDE1E6: low (metallic silver prints mid-grey) | Standard metallic silver; the screen silver #DDE1E6 is much lighter than the printed ink. |
| Silver | #DDE1E6 | PANTONE Cool Gray 2 C (kandydat do potwierdzenia wzornikiem) | solid ink, non-metallic | name: high; match: medium | Light cool grey for a printed (non-foil) silver. |
| Silver | #DDE1E6 | PANTONE Cool Gray 3 C (kandydat do potwierdzenia wzornikiem) | solid ink, non-metallic | name: high; match: medium | Slightly darker alternative to Cool Gray 2 C. |
| Gold | #C9A15A | PANTONE 871 C (kandydat do potwierdzenia wzornikiem) | metallic ink | name and type: high; match to #C9A15A: medium | Metallic gold of the Pantone Metallics range; usually darker and greener than the screen gold. |
| Gold | #C9A15A | PANTONE 874 C (kandydat do potwierdzenia wzornikiem) | metallic ink | name and type: medium; match: low | Neighbouring metallic gold of the same range; compare with 871 C on the fan deck. |
| Gold | #C9A15A | PANTONE 8003 C (kandydat do potwierdzenia wzornikiem) | metallic ink (named in the brief) | existence and hue: low, not verified | Taken over from the brief; check that it exists in the guide edition in use before it is quoted. |
| Gold | #C9A15A | PANTONE 7562 C (kandydat do potwierdzenia wzornikiem) | solid ink, non-metallic | name: medium; match: medium | Non-metallic gold-brown for a printed (non-foil) gold; no metallic lustre. |
| Ink | #2B2F36 | PANTONE 433 C (kandydat do potwierdzenia wzornikiem) | solid ink, non-metallic | name: medium; match: low | Dark blue-grey; check on the fan deck. |
| Ink | #2B2F36 | PANTONE 432 C (kandydat do potwierdzenia wzornikiem) | solid ink, non-metallic | name: medium; match: low | Slightly lighter blue-grey than 433 C. |
| Gold deep | #A3722A | PANTONE 872 C (kandydat do potwierdzenia wzornikiem) | metallic ink | name and type: medium; match: low | Darker metallic gold for use on light grounds. |
| Gold deep | #A3722A | PANTONE 875 C (kandydat do potwierdzenia wzornikiem) | metallic ink | name and type: medium; match: low | Bronze metallic; probably too brown, check on the fan deck. |
| Paper | #F6F7F8 | none | - | - | No candidate given: no certain Pantone name could be recalled for this colour. It is reproduced with the process recipe of this document, as a tint, or by the stock itself. |
| Paper warm | #F2EEE6 | none | - | - | No candidate given: no certain Pantone name could be recalled for this colour. It is reproduced with the process recipe of this document, as a tint, or by the stock itself. |
| Gold ground | #D9BC85 | none | - | - | No candidate given: no certain Pantone name could be recalled for this colour. It is reproduced with the process recipe of this document, as a tint, or by the stock itself. |

## 6. Millimetre conversions of the mark (certain, computed)

The mark box measures 67.00 x 77.279 units (outer extent of both chevrons) on the 96 unit grid. The height of the mark below is the height of this box; the 96 unit canvas is larger (1.2422 times the box height). Stroke 11 units, stream 5.5 units, inner corner radius 0.6 unit, outer corner radius 5.5 units.

| Mark height (mm) | Width (mm) | mm per unit | Stroke (mm) | Stream (mm) | Inner radius 0.6 unit (mm) | Radius >= 0.15 mm | Inner radius if the height is the 96 unit canvas (mm) | Emboss plateau width at 0.4 mm / 50 deg (mm) | Emboss channel floor between chevrons (mm) |
|---|---|---|---|---|---|---|---|---|---|
| 6 | 5.20 | 0.0776 | 0.85 | 0.43 | 0.047 | NO | 0.037 | 0.18 | -0.24 |
| 12 | 10.40 | 0.1553 | 1.71 | 0.85 | 0.093 | NO | 0.075 | 1.04 | 0.18 |
| 20 | 17.34 | 0.2588 | 2.85 | 1.42 | 0.155 | yes | 0.125 | 2.18 | 0.75 |
| 30 | 26.01 | 0.3882 | 4.27 | 2.14 | 0.233 | yes | 0.188 | 3.60 | 1.46 |

Wall run (horizontal extent of an emboss wall): 0.336 mm at depth 0.4 mm and 50 degrees; 0.500 mm at the worst case 0.5 mm and 45 degrees.

### Smallest mark height by rule (mm)

| Rule | Smallest mark height (mm) |
|---|---|
| Foil: stroke >= 0.25 mm | 1.76 |
| Foil: stream (clear gap) >= 0.4 mm | 5.62 |
| Foil: mark height >= 6 mm (rule of the brief) | 6.00 |
| Emboss: stroke >= 0.5 mm | 3.51 |
| Emboss: channel floor between chevrons stays open (nominal wall) | 9.43 |
| Emboss: channel floor between chevrons stays open (worst case wall) | 14.05 |
| Emboss: inner radius 0.6 unit >= 0.15 mm | **19.32** |

**Smallest embossed mark: 19.3 mm high (16.8 mm wide).** Of the reference heights, 12 mm fails the inner radius rule (0.093 mm), 20 mm passes (0.155 mm, margin 0.005 mm) and 30 mm passes (0.233 mm). Smallest foil mark: 6 mm high (5.2 mm wide).

## 7. Minimum sizes per layout (computed) and correction of the kit values

Features of the wordmark measured on the current outlines (cap height 100 units; gaps are exact distances between the letter polygons): thinnest line 10.13 units, narrowest counter 8.0 units. Gaps between neighbouring letters in units: Da 16.4, an 17.04, na 18.59, ac 18.11, co 13.34, Nu 17.38, ur 18.7, rt 10.68. The narrowest gap is the pair **rt (10.68 units)**; the next narrowest is co (13.34 units).

| Layout | Kit minimum, print (mm) | Ink, computed (mm) | Foil, computed (mm) | Emboss, computed (mm) | Emboss limited by |
|---|---|---|---|---|---|
| mark (height) | 6 | 6 | 6.0 | 19.3 | mark inner radius |
| horizontal (width) | 28 | 16.1 | 40.8 | 95.6 | mark inner radius |
| vertical (width) | 20 | 12.2 | 30.8 | 51.7 | channel |
| compact (width) | 20 | 7.6 | 20.2 | 52.0 | mark inner radius |
| wordmark (width) | 22 | 12.2 | 30.8 | 51.7 | channel |

Rules used: foil line >= 0.25 mm (0.7 pt), foil gap >= 0.4 mm, foil mark height >= 6 mm; emboss line >= 0.5 mm, inner radius >= 0.15 mm, channel floor between shapes open at 0.4 mm depth and 50 degrees (gap >= 0.67 mm); ink (offset) line and gap >= 0.15 mm (assumption of this document, confirm with the printer). The foil and emboss rules are those of the brief. The mark row gives heights, the other rows widths.

### Correction of the kit minimum sizes and reason

- Ink print: the computed values (16.1, 12.2, 7.6, 12.2 mm) are below the kit values, so the kit values (28, 20, 20, 22 mm; mark 6 mm) are kept. At the kit width of the horizontal lockup the narrowest letter gap (rt) is 0.275 mm.
- Foil: the kit values are too small for foil. The 0.4 mm gap rule applies to the pair rt (10.68 units): horizontal lockup 40.8 mm instead of 28 mm, vertical 30.8 mm instead of 20 mm, compact 20.2 mm instead of 20 mm, wordmark 30.8 mm instead of 22 mm. The mark stays at 6 mm.
- Emboss: new values, the kit has none. The mark is limited by the inner corner radius (19.3 mm high); the horizontal lockup inherits it (95.6 mm wide) because its mark must be that high; the vertical lockup and the wordmark alone are limited by the channel between letters or chevrons.


## 8. Hot foil requirements (description of requirements, no catalogue numbers)

- **process**: Hot foil stamping with a heated metal die; blind or registered emboss from a separate or combined die.
- **carrier**: Foil on a polyester carrier film with release layer, protective lacquer, metallised layer and adhesive layer; the adhesive grade must suit the stock (black board, cotton paper). A typical carrier thickness of 12 micrometres is common; the exact value is the printer's choice.
- **finish**: High gloss. Metallised foil for gold and silver. A pigment (opaque, non-metallic) gloss foil is only an option for a non-metallic colour; the brief names pigment gloss foil, which is confirmed or replaced by metallised foil with the printer.
- **gold**: Gold: warm yellow gold with a slight red component, neither lemon-green nor copper-red; lightness comparable to the brand gold #C9A15A (L* about 70) as a screen reference only. Ask for at least three gold swatches (pale, rich, warm) stamped on the real stock.
- **silver**: Silver: bright neutral to slightly cool silver, mirror gloss, no brushed or hologram effect unless ordered. Ask for at least two silver swatches stamped on the real stock.
- **catalogue**: No manufacturer, series or catalogue number is specified in this document. The choice is made from the swatches that the printer supplies; record the chosen foil type and number in the brand book after the first print.
- **sampling**: Request a stamping test on the actual stock (black board 400 to 600 g/m2 and cotton paper 300 to 600 g/m2) with the smallest mark size to be produced, before the order is released.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
