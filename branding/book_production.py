#!/usr/bin/env python3
# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
#
# Module:           branding/book_production.py
# Responsibility:   Write the identity guidelines of Danaco Nurt, edition 0.1.1, as a PDF of 1600 x 900 pages.
# Dependencies:     argparse, os, subprocess, sys, tempfile, pathlib, PIL, board, identity, mark, spec
# Author:           Danaco Official
# Version:          0.1.1, 2026-11-27
# License:          see the LICENSE file or https://www.danaco-group.pl

"""Write the identity guidelines of Danaco Nurt, edition 0.1.1, as a PDF of 1600 x 900 pages.

The edition describes the production identity 1.0.0 as shipped in release 0.1.1.
Every page is an SVG board of the kit of the design pack with the lettering
converted to curves, so the PDF does not depend on installed fonts. The images
come from the files of the pack and the banner drawings, the text and the
colours are those of the product (crates/nurt-contracts/src/product.rs and
crates/nurt-cli/src/theme.rs). The date of the PDF is fixed, so two runs on the
same sources write the same bytes.

Usage:

  book_production.py PACK [--out FILE] [--png-dir DIR]

PACK is the directory of the production pack (it holds backgrounds/, icons/,
lockups/, mockups/, render/ and build/kit/). The default FILE is
guidelines-0.1.1.pdf next to this script. The script needs the Python
environment of the pack (Pillow, fontTools, uharfbuzz, shapely, numpy) and the
program rsvg-convert; book.py starts it with --edition 0.1.1.
"""

import argparse
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = os.path.dirname(os.path.abspath(__file__))

PARSER = argparse.ArgumentParser(description="Write the identity guidelines, edition 0.1.1, as a PDF.")
PARSER.add_argument("pack", type=Path, help="directory of the production pack")
PARSER.add_argument("--out", type=Path, default=Path(HERE) / "guidelines-0.1.1.pdf")
PARSER.add_argument("--png-dir", type=Path, help="also write every page as a PNG into this directory")
ARGUMENTS = PARSER.parse_args()
PROD = str(ARGUMENTS.pack.resolve())
sys.path.insert(0, os.path.join(PROD, "build", "kit"))

from PIL import Image  # noqa: E402

import board as BD  # noqa: E402
import identity as ID  # noqa: E402
import mark as M  # noqa: E402
import spec as S  # noqa: E402

W, H = 1600, 900
BG, PANEL, LINE, MUTED, SOFT, WHITE, BODY = "#0B0C0E", "#111317", "#23262B", "#7C8390", "#9AA1AA", "#F6F7F8", "#C9CED5"
SILVER, GOLD, GOLD_DEEP, INK = S.TOKENS["silver"], S.TOKENS["gold"], S.TOKENS["gold_deep"], S.TOKENS["ink"]
CYAN = "#4FD1F5"
MX = 96
VERSION = "0.1.1"
TAGLINE = "Command shell and language where every change can be undone"
HINT = "Type 'help' to get started."
COPYRIGHT = "Copyright (c) 2026 Danaco Holding Group Sp. z o.o. All rights reserved."
# The fixed time of the PDF is 2026-11-27T00:00:00Z.
EPOCH = "1795737600"
TOKENS_GRAPHITE = S.TOKENS["graphite"]
TOKENS_PAPER = S.TOKENS["paper"]


def P(*parts):
    return os.path.join(PROD, *parts)


PAGES = []
_images = {}


def page(chapter, title=None, sub=None):
    def deco(fn):
        PAGES.append((chapter, title, sub, fn))
        return fn

    return deco


def card(b, x, y, w, h, fill=PANEL, stroke=LINE, rx=8):
    b.rect(x, y, w, h, fill=fill, stroke=stroke, rx=rx)


def label(b, x, y, s, size=13, fill=MUTED, weight=400, anchor="start", family="Geist", tracking=0.0):
    return b.text(x, y, s, size, fill, weight, anchor=anchor, family=family, tracking=tracking)


def para(b, x, y, s, width, size=16, fill=SOFT, weight=400, leading=1.5):
    return b.paragraph(x, y, s, size=size, width=width, fill=fill, weight=weight, leading=leading)


def bullets(b, x, y, width, items, size=16, fill=SOFT, gap=10, leading=1.45):
    for item in items:
        b.rect(x, y - size * 0.55, 5, 5, fill=GOLD)
        used = b.paragraph(x + 20, y, item, size=size, width=width - 20, fill=fill, leading=leading)
        y += used + gap
    return y


def table(b, x, y, widths, rows, size=13, head=True, pad=10, mono=(), first_bold=False):
    """Draw a table; returns the y below it. Cells wrap inside their column."""
    cy = y
    total = sum(widths)
    for r, row in enumerate(rows):
        cells = []
        lines_n = 1
        for i, cell in enumerate(row):
            family = "GeistMono" if i in mono else "Geist"
            weight = 600 if (head and r == 0) or (first_bold and i == 0) else 400
            lines = BD.wrap(str(cell), size, widths[i] - 2 * pad, weight, family=family)
            cells.append((lines, family, weight))
            lines_n = max(lines_n, len(lines))
        rh = lines_n * size * 1.4 + 16
        if head and r == 0:
            b.rect(x, cy, total, rh, fill="#15171A")
        cx = x
        for i, (lines, family, weight) in enumerate(cells):
            colour = GOLD if (head and r == 0) else (WHITE if (first_bold and i == 0) else BODY)
            for k, line in enumerate(lines):
                b.text(cx + pad, cy + 8 + size + k * size * 1.4 - 2, line, size, colour, weight, family=family)
            cx += widths[i]
        cy += rh
        b.line(x, cy, x + total, cy, stroke=LINE)
    return cy


def picture(path, w, h, cover, scale=1.3, crop=None):
    key = (path, int(w), int(h), cover, crop)
    if key in _images:
        return _images[key]
    im = Image.open(path).convert("RGBA")
    if crop:
        im = im.crop((round(crop[0] * im.width), round(crop[1] * im.height), round(crop[2] * im.width), round(crop[3] * im.height)))
    tw, th = int(w * scale), int(h * scale)
    if cover:
        s = max(tw / im.width, th / im.height)
        im = im.resize((max(1, round(im.width * s)), max(1, round(im.height * s))), Image.LANCZOS)
        left, top = (im.width - tw) // 2, (im.height - th) // 2
        im = im.crop((left, top, left + tw, top + th))
    else:
        s = min(tw / im.width, th / im.height, 1.0)
        im = im.resize((max(1, round(im.width * s)), max(1, round(im.height * s))), Image.LANCZOS)
    _images[key] = im
    return im


def pic(b, rel, x, y, w, h, cover=False, radius=8, frame=True, ground=None, crop=None):
    """Place the picture `rel` (path inside the pack) in the cell (x, y, w, h); crop is (left, top, right, bottom) in fractions."""
    path = P(*rel.split("/"))
    im = picture(path, w, h, cover, crop=crop)
    if ground:
        b.rect(x, y, w, h, fill=ground, rx=radius)
    if cover:
        b.image(im, x, y, w, h, clip_radius=radius)
    else:
        dw = min(w, h * im.width / im.height)
        dh = dw * im.height / im.width
        b.image(im, x + (w - dw) / 2, y + (h - dh) / 2, dw, dh, clip_radius=radius)
    if frame:
        b.rect(x, y, w, h, stroke=LINE, rx=radius)


def logo_in(b, layout, variant, x, y, w, h, pad=0.14):
    """Place a layout, fitted into the box (x, y, w, h) and centred, on the ground of the variant."""
    spec = ID.VARIANTS[variant]
    if variant.startswith("emboss"):
        b.rect(x, y, w, h, fill=spec["ground"], stroke=LINE, rx=8)
        pic(b, f"lockups/lockup-{layout}-{variant}.png", x + w * pad, y + h * pad, w * (1 - 2 * pad), h * (1 - 2 * pad), radius=0, frame=False)
        return
    b.rect(x, y, w, h, fill=spec["ground"], stroke=LINE, rx=8)
    box = (w * (1 - 2 * pad), h * (1 - 2 * pad))
    tmp = BD.Board(10, 10)
    pw, ph = tmp.logo(layout, variant, 0, 0, box=box)
    b.logo(layout, variant, x + (w - pw) / 2, y + (h - ph) / 2, box=box)




def frame(b, index, total, chapter, title, sub):
    if chapter:
        number, name = chapter
        b.text(MX, 92, number, 14, GOLD, 500, tracking=60)
        b.text(MX + 46, 92, name.upper(), 14, SOFT, 500, tracking=60)
        if title:
            b.text(MX, 150, title, 38, WHITE, 600, tracking=-12)
        if sub:
            b.text(MX, 184, sub, 17, SOFT, 400)
    b.line(MX, H - 58, W - MX, H - 58, stroke=LINE)
    label(b, MX, H - 30, "Danaco Nurt  ·  Identity guidelines  ·  production identity 1.0.0, release " + VERSION, 12)
    label(b, W - MX, H - 30, f"{index} / {total}", 12, anchor="end")




@page(None)
def cover(b):
    pic(b, "backgrounds/background-stream-dark.png", 0, 0, W, H, cover=True, radius=0, frame=False)
    b.rect(0, 0, W, H, fill="#0B0C0E", opacity=0.35)
    b.logo("horizontal", "metal", MX, 300, width=720)
    b.text(MX, 560, "Identity guidelines", 54, WHITE, 600, tracking=-14)
    b.text(MX, 604, "Production identity 1.0.0 for release " + VERSION, 22, SOFT, 400)
    b.text(MX, 650, "The mark, the name, the colours, the grounds, the finishing, the print, the icons and the terminal.", 17, MUTED, 400)


@page(None)
def contents(b):
    b.text(MX, 150, "Contents", 38, WHITE, 600, tracking=-12)
    seen, first = [], {}
    for index, (chapter, _t, _s, _f) in enumerate(PAGES, start=1):
        if chapter and chapter not in first:
            first[chapter] = index
            seen.append(chapter)
    col_x = [MX, MX + 740]
    for n, chapter in enumerate(seen):
        col, row = divmod(n, 6)
        x, y = col_x[col], 250 + row * 86
        b.text(x, y, chapter[0], 14, GOLD, 500, tracking=60)
        b.text(x + 56, y, chapter[1], 28, WHITE, 500, tracking=-8)
        b.text(x + 560, y, f"{first[chapter]}", 16, SOFT, 400, anchor="end")
        b.line(x, y + 24, x + 560, y + 24, stroke=LINE)


@page(("01", "Idea"), "What changed", "The production identity replaces the identity before production")
def changed(b):
    rows = [
        ["Area", "Before", "Production 1.0.0", "Reason"],
        ["Arms of the mark", "36.5 degrees", "36.87 degrees, slope 3 : 4", "A ratio of whole numbers: every point is set with a ruler and a compass."],
        ["Stroke : stream", "11 : 5 nominal, 5.9 clear, unequal along the band", "11 : 5.5, equal along the whole band", "The stream is half of the stroke and has one width everywhere; at 16 px the 5.9 clearance measured 1.34 px."],
        ["Cuts of the arms", "x = 45.5, tips 5.0 units apart", "x = 49.7, tips interlock by 3.4 units", "No vertical line divides the figure; a plain pair <> is always divided by one."],
        ["Tones", "silver #C3C8CE, gold #D4AE6E: 1.24 : 1", "silver #DDE1E6, gold #C9A15A: 1.83 : 1", "The pair keeps a lightness difference in grey (1.91 : 1 against 1.27 : 1)."],
        ["Small sizes", "one drawing for 16 to 32 px", "three drawings: 16, 24 and 32 px", "Aligned to the pixel grid; the channel stays open at every size."],
        ["Typeface of the name", "letters drawn on the mark grid", "Geist 400 / 700, own spacing per pair, bevels on N, r, t", "Flat cut terminals take the cuts of the mark; Geist Mono is the companion for the terminal."],
        ["Layouts", "horizontal, stacked, mark", "horizontal, vertical, compact, mark, wordmark", "Each in nine colour variants, ten with the screen metal (the wordmark has no metal variant)."],
        ["Grounds", "graphite and light", "seven grounds with a measured matrix of pairings", "Contrast measured, not assumed."],
        ["Terminal banner", "the name in large letters", "the mark in half blocks; 7-bit ASCII without UTF-8", "The banner shows the figure of the product in its two tones."],
    ]
    table(b, MX, 225, [210, 330, 400, 468], rows, size=13, first_bold=True)


@page(("01", "Idea"), "The idea", "A step and its undo are one figure")
def idea(b):
    b.mark_drawing(S.MAIN, 96, 240, 470)
    bx = 640
    para(b, bx, 262, "Two identical chevrons. The left one is silver and points back: the step back, undo. The right one is gold and points forward: the step forward, the change. The gold chevron is the silver one turned by 180 degrees about the centre of the grid, so the two are one figure.", 760, 19, WHITE)
    para(b, bx, 410, "The channel between the two parallel arms is the stream, in Polish nurt. It has one width along the whole band, half of the stroke.", 760, 17)
    para(b, bx, 490, "Nurt is a command shell and a language in which every change is recorded in a journal and can be undone. The mark says it without a word: the silver chevron is always on the left and raised, the gold chevron on the right and lowered.", 760, 17)
    card(b, bx, 620, 760, 120)
    b.text(bx + 24, 665, "Silver  #DDE1E6", 16, SILVER, 500)
    b.text(bx + 24, 696, "the step back", 14, MUTED)
    b.text(bx + 280, 665, "Gold  #C9A15A", 16, GOLD, 500)
    b.text(bx + 280, 696, "the step forward", 14, MUTED)
    b.text(bx + 540, 665, "Cyan  #4FD1F5", 16, CYAN, 500)
    b.text(bx + 540, 696, "the blue of the terminal, text only", 14, MUTED)


@page(("02", "Mark"), "Construction", "Grid of 96 units, slope 3 : 4, stroke 11, stream 5.5")
def construction(b):
    gx, gy, gs = MX, 215, 570
    k = gs / 96.0
    b.rect(gx, gy, gs, gs, fill="#0E1013", stroke=LINE, rx=4)
    for u in (0, 24, 48, 72, 96):
        b.line(gx + u * k, gy, gx + u * k, gy + gs, stroke="#1B1E23")
        b.line(gx, gy + u * k, gx + gs, gy + u * k, stroke="#1B1E23")
    b.line(gx + 48 * k, gy, gx + 48 * k, gy + gs, stroke="#3A3F47", dash="6 6")
    b.line(gx, gy + 48 * k, gx + gs, gy + 48 * k, stroke="#3A3F47", dash="6 6")
    x0, y0, x1, y1 = M.bbox(S.MAIN)
    b.rect(gx + x0 * k, gy + y0 * k, (x1 - x0) * k, (y1 - y0) * k, stroke="#3A3F47")
    b.mark_drawing(S.MAIN, gx + x0 * k, gy + y0 * k, (y1 - y0) * k)
    b.line(gx + 49.7 * k, gy + 6, gx + 49.7 * k, gy + gs - 6, stroke=SILVER, dash="3 5", opacity=0.7)
    b.line(gx + 46.3 * k, gy + 6, gx + 46.3 * k, gy + gs - 6, stroke=GOLD, dash="3 5", opacity=0.7)
    label(b, gx, gy + gs + 26, "Grid 96 x 96 units, centre of the turn (48, 48). Dashed: cuts at x = 49.7 and x = 46.3.", 13)
    rows = [
        ["Parameter", "Value"],
        ["Grid", "96 x 96 units, centre of the turn (48, 48)"],
        ["Slope of the arms", "3 : 4 (36.8699 degrees)"],
        ["Stroke", "11 units"],
        ["Stream (clearance between the chevrons)", "5.5 units, equal along the band (measured 5.50)"],
        ["Apex of the silver chevron", "x = 20, centre line at y = 37.31"],
        ["Cuts of the arms", "vertical, x = 49.7 (silver) and x = 46.3 (gold)"],
        ["Radii", "outer apex 5.5 (half of the stroke), four ends 1.2, inner apex 0.6"],
        ["Box of the mark", "67.0 x 77.28 units"],
        ["Interlock of the tips", "3.4 units"],
    ]
    y = table(b, 740, 225, [300, 464], rows, size=14, first_bold=True)
    para(b, 740, y + 34, "The gold chevron is the silver one rotated by 180 degrees about (48, 48); both are drawn from one list of vertices, so the turn is exact. Corners are circular arcs written as cubic curves. The paths are in identity.json.", 764, 15)


@page(("02", "Mark"), "Decisions of the mark", "What the production drawing corrects")
def decisions(b):
    pic(b, "board-before-after.png", MX, 215, 1000, 317, crop=(0.0, 0.095, 1.0, 0.30), radius=8)
    label(b, MX, 560, "Left: the identity before production. Right: production 1.0.0; dashed, the former construction on the same grid.", 13)
    bullets(b, 1150, 250, 354, [
        "The arms follow 3 : 4, so every point can be set with a ruler and a compass.",
        "The stream is half of the stroke and equal along the band; at 16 px the former clearance measured 1.34 px.",
        "The cuts pass the axis: the gold tip enters the mouth of the silver chevron by 3.4 units, and no vertical line divides the figure. This weakens the association with a plain pair of brackets.",
        "Silver and gold differ in lightness: 1.83 : 1, in grey 1.91 : 1 (before 1.27 : 1).",
    ], size=14)


@page(("02", "Mark"), "Drawings for small sizes", "Three drawings aligned to the pixel grid; the main drawing from 48 px")
def small_sizes(b):
    items = [("S16", "16 to 23 px", S.S16), ("S24", "24 to 31 px", S.S24), ("S32", "32 to 47 px", S.S32), ("Main", "from 48 px", S.MAIN)]
    for i, (name, used, spec) in enumerate(items):
        x = MX + i * 358
        card(b, x, 215, 340, 230)
        x0, y0, x1, y1 = M.bbox(spec)
        height = 150
        wd = (x1 - x0) * height / (y1 - y0)
        b.mark_drawing(spec, x + (340 - wd) / 2, 240, height)
        b.text(x + 20, 428, name, 15, WHITE, 600)
        b.text(x + 320, 428, used, 13, MUTED, anchor="end")
    label(b, MX, 490, "At real size (one pixel of the page is one pixel of the screen):", 13)
    px = MX
    for name, spec, size in (("16 px", S.S16, 16), ("24 px", S.S24, 24), ("32 px", S.S32, 32), ("48 px", S.MAIN, 48), ("96 px", S.MAIN, 96)):
        x0, y0, x1, y1 = M.bbox(spec)
        wd = (x1 - x0) * size / (y1 - y0)
        b.mark_drawing(spec, px, 600 - size, size)
        label(b, px, 622, name, 12)
        px += max(wd, 56) + 50
    rows = [
        ["Drawing", "Used at", "Stroke", "Stream", "Cuts"],
        ["S16", "16 to 23 px", "3.00 px", "2.00 px", "x = 42 and 54"],
        ["S24", "24 to 31 px", "3.50 px", "2.11 px", "x = 44 and 52"],
        ["S32", "32 to 47 px", "4.00 px", "2.00 px", "x = 48 and 48"],
        ["Main", "from 48 px", "5.50 px at 48 px", "2.75 px at 48 px", "x = 49.7 and 46.3"],
    ]
    y = table(b, MX, 650, [200, 260, 320, 320, 308], rows, size=13, first_bold=True)
    label(b, MX + 760, 622, "No interlock in the small drawings: a fraction of a pixel would blur. Square ends, outer apex rounded, inner apex sharp.", 12)


@page(("02", "Mark"), "Clear space and minimum size", "A quarter of the box height around the mark; the cap height around a lockup")
def clear_space(b):
    card(b, MX, 215, 560, 340)
    x0, y0, x1, y1 = M.bbox(S.MAIN)
    mh = 150
    mw = (x1 - x0) * mh / (y1 - y0)
    mx, my = MX + (560 - mw) / 2, 215 + (340 - mh) / 2
    q = mh / 4
    b.rect(mx - q, my - q, mw + 2 * q, mh + 2 * q, stroke=GOLD)
    b.rect(mx, my, mw, mh, stroke="#3A3F47")
    b.mark_drawing(S.MAIN, mx, my, mh)
    label(b, MX, 580, "Mark alone: a quarter of the box height (19 units) on every side.", 13)
    card(b, 700, 215, 804, 340, fill=BG)
    tmp = BD.Board(10, 10)
    lw, lh = tmp.logo("horizontal", "color-dark", 0, 0, width=520)
    lx, ly = 700 + (804 - lw) / 2, 215 + (340 - lh) / 2
    cap = lh / 2.2 / 2.2
    b.rect(lx - cap, ly - cap, lw + 2 * cap, lh + 2 * cap, stroke=GOLD)
    b.logo("horizontal", "color-dark", lx, ly, width=520)
    label(b, 700, 580, "Lockups: the cap height of the wordmark (x) on every side.", 13)
    rows = [
        ["Layout", "Screen", "Ink in print", "Hot foil", "Blind emboss"],
        ["mark", "16 px (S16), 24 px (S24), 32 px (S32), 48 px main", "6 mm", "6.0 mm", "19.3 mm"],
        ["horizontal", "96 px", "28 mm", "40.8 mm", "95.6 mm"],
        ["vertical", "64 px", "20 mm", "30.8 mm", "51.7 mm"],
        ["compact", "56 px", "20 mm", "20.2 mm", "52.0 mm"],
        ["wordmark", "72 px", "22 mm", "30.8 mm", "51.7 mm"],
    ]
    table(b, MX, 615, [200, 520, 220, 220, 248], rows, size=12.5, first_bold=True)


@page(("03", "Wordmark"), "The name", "Geist: Danaco in regular, Nurt in bold")
def wordmark(b):
    card(b, MX, 215, 1408, 260, fill=BG)
    b.logo("wordmark", "color-dark", MX + 240, 290, width=930)
    para(b, MX, 520, "Eight typefaces, all under the SIL Open Font License 1.1, were set at one cap height in 400 and 700 with the font kerning only: JetBrains Mono, Inter, Instrument Sans, Manrope, Outfit, Sora, Plus Jakarta Sans and Geist. Criteria: Polish diacritics, weight range, flat cut terminals that take the cuts of the mark, and a companion for the terminal.", 680, 15)
    para(b, 840, 520, "Geist is a neutral geometric grotesque with flat, square-cut terminals on r and t and a plain N, so that a cut at the slope of the mark reads as a deliberate gesture. Its monospaced sibling Geist Mono is the face of the terminal and of code. Rejected: Outfit and Sora (round forms), Manrope and Plus Jakarta Sans (soft terminals), Inter (wide default spacing), JetBrains Mono (a code-editor look).", 664, 15)
    para(b, MX, 700, "The company name steps back, the product name steps forward: Danaco regular (stem 11.8 cap units), Nurt bold (stem 21.1), ratio 1 : 1.79.", 1300, 16, WHITE)


@page(("03", "Wordmark"), "Setting of the wordmark", "Tracking, pair corrections and bevels")
def setting(b):
    rows = [
        ["Parameter", "Value"],
        ["Weights", "Danaco 400, Nurt 700"],
        ["Tracking (per mille of the em)", "Danaco +22, Nurt -4"],
        ["Pair corrections (per mille of the em, on top of the font kerning)", "Da -10, an +4, na -4, ac +12, co -2, Nu +6, ur +6, rt +34"],
        ["Gap between the words", "40 cap units (cap height = 100)"],
        ["Bevels on N, r, t", "100 per mille of the em, at 3 : 4"],
        ["Width of the wordmark", "823.1 cap units; x-height 74.6"],
        ["Closest approach of neighbouring letters (cap units)", "Da 16.4, an 17.0, na 18.6, ac 18.1, co 13.3, Nu 17.4, ur 18.7, rt 10.7"],
    ]
    y = table(b, MX, 225, [420, 640], rows, size=14, first_bold=True)
    card(b, 1220, 225, 284, 260, fill=BG)
    b.logo("wordmark", "color-dark", 1236, 335, width=252)
    para(b, MX, y + 40, "N, r and t receive a cut at the slope of the arms of the mark: N on its top right and bottom left corners (the letter keeps its 180 degree symmetry, as the mark does), r on the terminal, t on the top of the stem. Counters, stems and curves stay as designed.", 1300, 15)
    para(b, MX, y + 120, "The pair rt was corrected by hand to +34: the terminal of r and the crossbar of t came within 4.8 cap units; after the correction the closest approach is 10.7, the narrowest gap of the wordmark and the value that decides the minimum widths in hot foil. Every pair was verified by exact polygon distance.", 1300, 15)


@page(("04", "Layouts"), "Layouts", "Horizontal, vertical, compact, mark and wordmark")
def layouts(b):
    specs = [
        ("horizontal", "Horizontal", "Mark 2.2 cap heights tall, gap 0.75 cap; the centre of the turn 3 cap units below the middle of the cap height.", "96 px"),
        ("vertical", "Vertical", "Mark 3.8 cap heights tall, gap 0.95; the wordmark centred under the mark.", "64 px"),
        ("compact", "Compact", "Mark 2.0 cap heights tall, gap 0.68; the name Nurt alone.", "56 px"),
    ]
    widths = [560, 380, 420]
    x = MX
    for (layout, name, text, minimum), w in zip(specs, widths):
        logo_in(b, layout, "color-dark", x, 215, w, 320, pad=0.16)
        b.text(x, 572, name, 18, WHITE, 600)
        para(b, x, 600, text, w - 20, 14)
        label(b, x, 690, f"Minimum width on screen: {minimum}", 13)
        x += w + 24
    para(b, MX, 750, "The mark alone and the wordmark alone are the two other layouts (minimum 16 px by height with S16, and 72 px wide). Use the supplied files; do not redraw or retype the mark or the name.", 1300, 15)


@page(("04", "Layouts"), "Colour variants", "Nine variants of every layout, ten with the screen metal")
def variants(b):
    names = [
        ("color-dark", "graphite, deep black, graphite with the trace"), ("color-light", "paper, warm paper, paper with the trace"),
        ("black", "paper, warm paper, gold ground; one-colour print"), ("white", "graphite, deep black; engraving, one-colour print"),
        ("grey", "grey print, photocopies"), ("grey-dark", "grey screens"), ("gold", "deep black and graphite; foil stamping"),
        ("metal", "screens from 128 px; flat lettering"), ("emboss-light", "blind emboss mockups, cotton paper"), ("emboss-dark", "blind emboss on dark card"),
    ]
    cw, ch = 264, 160
    for i, (variant, use) in enumerate(names):
        col, row = i % 5, i // 5
        x, y = MX + col * (cw + 22), 215 + row * (ch + 110)
        logo_in(b, "horizontal", variant, x, y, cw, ch, pad=0.12)
        b.text(x, y + ch + 26, variant, 14, WHITE, 600, family="GeistMono")
        para(b, x, y + ch + 46, use, cw, 12.5, MUTED, leading=1.35)
    para(b, MX, 760, "Layouts shown: horizontal. The vertical, compact, mark and wordmark layouts exist in the same variants, except that the wordmark has no metal variant. The emboss variants are plates for mockups, not artwork for print.", 1300, 14)


@page(("05", "Colour"), "Colour tokens", "Names describe the function")
def colour(b):
    tokens = [
        ("graphite", "#0B0C0E", "11 12 14", "ground, main"), ("graphite_soft", "#15171A", "21 23 26", "ground, soft"), ("black_deep", "#050506", "5 5 6", "ground, deep black"),
        ("silver", "#DDE1E6", "221 225 230", "step back"), ("gold", "#C9A15A", "201 161 90", "step forward"),
        ("ink", "#2B2F36", "43 47 54", "ink on light"), ("gold_deep", "#A3722A", "163 114 42", "gold on light"),
        ("paper", "#F6F7F8", "246 247 248", "ground, light"), ("paper_warm", "#F2EEE6", "242 238 230", "ground, warm light"), ("gold_ground", "#D9BC85", "217 188 133", "ground, gold"),
    ]
    cw, ch = 264, 150
    for i, (name, hexv, rgb, role) in enumerate(tokens):
        col, row = i % 5, i // 5
        x, y = MX + col * (cw + 22), 215 + row * (ch + 80)
        b.rect(x, y, cw, ch, fill=hexv, stroke="#3A3F47", rx=8)
        b.text(x, y + ch + 24, name, 15, WHITE, 600, family="GeistMono")
        label(b, x, y + ch + 44, f"{hexv}   {rgb}", 13, fill=BODY, family="GeistMono")
        label(b, x, y + ch + 62, role, 12.5)
    para(b, MX, 700, "Additional lettering tones: Danaco on dark #C9CED5, Nurt on dark #F6F7F8, Danaco on light #4A515C, Nurt on light #0B0C0E.", 1300, 14)
    para(b, MX, 735, "Cyan #4FD1F5 (palette index 81) is the blue of the terminal: the strings of a typed line. It is not a colour of the logo.", 1300, 14)


@page(("05", "Colour"), "Colour for print", "CMYK from ICC profiles; Pantone numbers are candidates")
def print_colour(b):
    rows = [
        ["Token", "HEX", "CMYK coated FOGRA39", "CMYK uncoated FOGRA47", "Pantone"],
        ["graphite", "#0B0C0E", "60 / 50 / 50 / 100", "50 / 40 / 40 / 100", "Black 6 C (candidate)"],
        ["graphite_soft", "#15171A", "60 / 50 / 50 / 100", "50 / 40 / 40 / 100", "none (stock or tint)"],
        ["black_deep", "#050506", "60 / 50 / 50 / 100", "50 / 40 / 40 / 100", "none (stock or tint)"],
        ["silver", "#DDE1E6", "16 / 9 / 8 / 0", "13 / 7 / 6 / 1", "877 C metallic; Cool Gray 2 C printed (candidates)"],
        ["gold", "#C9A15A", "20 / 34 / 70 / 8", "9 / 28 / 70 / 13", "871 C metallic; 7562 C printed (candidates)"],
        ["ink", "#2B2F36", "75 / 63 / 51 / 65", "73 / 53 / 37 / 74", "433 C (candidate, low certainty)"],
        ["gold_deep", "#A3722A", "28 / 50 / 89 / 22", "14 / 46 / 99 / 24", "872 C metallic (candidate, low certainty)"],
        ["paper", "#F6F7F8", "4 / 2 / 3 / 0", "3 / 2 / 2 / 0", "none (stock or tint)"],
        ["paper_warm", "#F2EEE6", "6 / 6 / 11 / 0", "5 / 5 / 8 / 1", "none (stock or tint)"],
        ["gold_ground", "#D9BC85", "16 / 25 / 53 / 3", "9 / 19 / 48 / 8", "none (stock or tint)"],
    ]
    y = table(b, MX, 215, [180, 150, 300, 300, 478], rows, size=12.5, first_bold=True, mono=(1,))
    para(b, MX, y + 28, "CMYK is computed with transicc (LittleCMS 2.17), relative colorimetric intent with black point compensation; profiles ISO Coated v2 (FOGRA39, TAC limit 300) and PSO Uncoated ISO12647 (FOGRA47, TAC limit 260). Graphite prints as rich black: 60 / 50 / 50 / 100 (TAC 260) on coated, 50 / 40 / 40 / 100 (TAC 230) on uncoated.", 1300, 14)
    para(b, MX, y + 96, "Pantone entries are candidates to be confirmed against a swatch book at the first print; none is certain and none is quoted as a decision.", 1300, 14)


@page(("05", "Colour"), "Metals and contrast", "Screen gradients from 128 px; measured contrast of the tones")
def metals(b):
    silver = [(0.0, "#F7F9FB"), (0.16, "#C3C9D1"), (0.36, "#EEF1F4"), (0.58, "#9AA1AA"), (0.8, "#D8DCE2"), (1.0, "#8A919A")]
    gold = [(0.0, "#F6E3B4"), (0.22, "#D6AE66"), (0.46, "#B9893F"), (0.7, "#F0D6A0"), (1.0, "#A9792F")]
    foil = [(0.0, "#EBD29B"), (0.22, "#CFA45C"), (0.45, "#F2DBA6"), (0.7, "#B7863F"), (1.0, "#D9B672")]
    for i, (name, stops) in enumerate((("Silver, brushed (6 stops)", silver), ("Gold, polished (5 stops)", gold), ("Gold foil, single-colour variant (5 stops)", foil))):
        y = 225 + i * 135
        gid = f"mg{i}"
        b.add_defs(f'<linearGradient id="{gid}" x1="0" y1="0" x2="1" y2="0">' + "".join(f'<stop offset="{o}" stop-color="{c}"/>' for o, c in stops) + "</linearGradient>")
        b.rect(MX, y, 760, 60, fill=f"url(#{gid})", rx=6)
        b.text(MX, y + 88, name, 15, WHITE, 600)
        label(b, MX, y + 108, "  ".join(f"{o}: {c}" for o, c in stops), 11, fill=MUTED, family="GeistMono")
    rows = [
        ["Pair", "Contrast (WCAG 2.1)"],
        ["silver on graphite", "14.9 : 1"],
        ["gold on graphite", "8.1 : 1"],
        ["ink on paper", "12.5 : 1"],
        ["gold deep on paper", "3.9 : 1 (mark and lines only, not text)"],
        ["silver : gold", "1.83 : 1"],
        ["ink : gold deep", "3.20 : 1"],
    ]
    table(b, 930, 225, [270, 304], rows, size=14, first_bold=True)
    para(b, 930, 520, "A variant is allowed on a ground when the mark keeps 3 : 1 and the lettering 4.5 : 1 against it.", 574, 14)
    para(b, 930, 580, "The metal is flat premium metal for large formats and hero surfaces, with a lit edge line of 0.55 units. The lettering stays flat; below 64 px and in the product the flat tones are used.", 574, 14)


@page(("06", "Grounds"), "Grounds", "Seven grounds, 1920 x 1080 and 2560 x 1440")
def grounds(b):
    items = [
        ("background-graphite", "Graphite", "#0B0C0E", "main ground of the product: terminal, documentation, screens"),
        ("background-black-deep", "Deep black", "#050506", "foil and metal in print, presentations"),
        ("background-paper", "Light", "#F6F7F8", "documents, papers, light screens"),
        ("background-paper-warm", "Warm light", "#F2EEE6", "stationery, packaging, emboss"),
        ("background-gold", "Warm gold", "#D9BC85", "campaign grounds; only the black one-colour variant"),
        ("background-stream-dark", "Graphite with the trace", "#0B0C0E", "covers, social cards, headers of documentation"),
        ("background-stream-light", "Light with the trace", "#F6F7F8", "light covers and headers"),
    ]
    cw, ch = 316, 178
    for i, (file, name, hexv, use) in enumerate(items):
        col, row = i % 4, i // 4
        x, y = MX + col * (cw + 48), 215 + row * (ch + 130)
        pic(b, f"backgrounds/{file}.png", x, y, cw, ch, cover=True)
        b.text(x, y + ch + 26, name, 15, WHITE, 600)
        label(b, x, y + ch + 46, hexv, 12.5, fill=BODY, family="GeistMono")
        para(b, x, y + ch + 66, use, cw, 12.5, MUTED, leading=1.35)
    para(b, MX + 3 * (cw + 48), 215 + ch + 130 + 4, "The trace of the stream is a set of hairlines at the slope 3 : 4 that fade out toward the centre, with one pair of lines, silver and gold, spaced by the stream of the mark. A repeatable tile for CSS and print (96 px) exists in the two themes.", cw, 13, SOFT)


@page(("06", "Grounds"), "Pairings", "Which variant on which ground, measured")
def pairings(b):
    head = ["Variant", "graphite", "deep black", "light", "warm light", "gold", "graphite + trace", "light + trace"]
    data = [
        ("color-dark", "yes (8.1)", "yes (8.5)", "no (1.0)", "no (1.1)", "no (1.2)", "yes (8.1)", "no (1.0)"),
        ("color-light", "no (1.0)", "no (1.0)", "yes (3.9)", "yes (3.6)", "no (2.3)", "no (1.0)", "yes (3.9)"),
        ("black", "no (1.1)", "no (1.0)", "yes (19.6)", "yes (18.1)", "yes (11.5)", "no (1.1)", "yes (19.6)"),
        ("white", "yes (19.6)", "yes (20.4)", "no (1.1)", "no (1.2)", "no (1.8)", "yes (19.6)", "no (1.1)"),
        ("grey", "no (1.1)", "no (1.1)", "yes (4.2)", "yes (3.9)", "no (2.5)", "no (1.1)", "yes (4.2)"),
        ("grey-dark", "yes (7.8)", "yes (8.2)", "no (1.0)", "no (1.1)", "no (1.1)", "yes (7.8)", "no (1.0)"),
        ("gold", "yes (6.0)", "yes (6.3)", "no (3.0)", "no (2.8)", "no (1.8)", "yes (6.0)", "no (3.0)"),
        ("metal", "yes", "yes", "no", "no", "no", "yes", "no"),
        ("emboss-light", "no", "no", "yes", "yes", "no", "no", "no"),
        ("emboss-dark", "yes", "yes", "no", "no", "no", "no", "no"),
    ]
    widths = [200, 170, 170, 170, 170, 170, 180, 178]
    x, y = MX, 215
    b.rect(x, y, sum(widths), 40, fill="#15171A")
    cx = x
    for h, w in zip(head, widths):
        b.text(cx + 12, y + 26, h, 13, GOLD, 600)
        cx += w
    y += 40
    for row in data:
        cx = x
        for i, (cell, w) in enumerate(zip(row, widths)):
            if i == 0:
                b.text(cx + 12, y + 28, cell, 14, WHITE, 600, family="GeistMono")
            else:
                ok = cell.startswith("yes")
                if ok:
                    b.rect(cx + 6, y + 6, w - 12, 34, fill="#1A1710", rx=5)
                b.text(cx + 16, y + 28, cell, 13.5, GOLD if ok else MUTED, 500 if ok else 400)
            cx += w
        y += 46
        b.line(x, y, x + sum(widths), y, stroke=LINE)
    para(b, MX, y + 28, "A variant is allowed on a ground when the mark keeps 3 : 1 and the lettering 4.5 : 1 against the ground (WCAG 2.1); the number is the contrast. The metal and emboss variants follow the material rule.", 1300, 14)


@page(("07", "Finishing"), "Finishing for screens", "Flat premium metal and 3D renders")
def finishing(b):
    logo_in(b, "horizontal", "metal", MX, 215, 640, 300, pad=0.14)
    pic(b, "render/render-hero-front.png", 760, 215, 744, 418, cover=True)
    para(b, MX, 548, "Flat premium metal: silver brushed (6 stops, fine horizontal grain), gold polished (5 stops), a lit edge line of 0.55 units. For large formats and hero surfaces from 128 px; the lettering stays flat; below 64 px and in the product the flat tones are used.", 640, 14.5)
    para(b, 760, 660, "3D renders: Blender 5.2.1 LTS, Cycles, CPU, OpenImageDenoise. Brushed silver (metal 1.0, roughness 0.28 to 0.38, anisotropy 0.6 to 0.8 along the brushing) and polished gold (roughness 0.16 to 0.24); silver is brighter than gold in every view. Extrusion 8 units of the 96 grid, bevel 0.6 units in three segments. A studio HDRI (CC0) is used for reflections only.", 744, 14.5)


@page(("07", "Finishing"), "Renders", "Five renders; the parameters are stored with them")
def renders(b):
    items = [
        ("render/render-hero-front.png", "render-hero-front", "2560 x 1440, 192 samples, lens 120 mm, f/11"),
        ("render/render-hero-angle.png", "render-hero-angle", "2560 x 1440, 128 samples, lens 100 mm, f/2.8"),
        ("render/render-macro-edge.png", "render-macro-edge", "2560 x 1440, 256 samples, lens 100 mm, f/5.6"),
        ("render/render-light-ground.png", "render-light-ground", "2560 x 1440, 192 samples, lens 120 mm, f/11"),
    ]
    cw, ch = 686, 212
    for i, (rel, name, note) in enumerate(items):
        col, row = i % 2, i // 2
        x, y = MX + col * (cw + 36), 215 + row * (ch + 80)
        pic(b, rel, x, y, cw, ch, cover=True)
        b.text(x, y + ch + 24, name, 14, WHITE, 600, family="GeistMono")
        label(b, x, y + ch + 44, note, 12.5)
    para(b, MX, 806, "render-hero-transparent.png (2048 x 2048, RGBA, 192 samples) carries the mark on a transparent ground. Colour transform: Khronos PBR Neutral.", 1300, 13.5)


@page(("08", "Print"), "Print specification", "Foil, blind emboss and the preparation of files")
def print_spec(b):
    cols = [
        ("Separations", "Vector PDF 1.4, one page, trimmed to the logo with a margin of 3 mm; spot colours written as Separation: FOIL_GOLD, FOIL_SILVER, EMBOSS_DIE, K 100 %, and a CMYK version for a dark ground. For the layouts horizontal, vertical and mark."),
        ("Hot foil", "Heated die, high-gloss metallised foil on a polyester carrier. Gold: warm yellow gold with a slight red component (screen reference #C9A15A); silver: bright neutral to slightly cool mirror silver. Line 0.25 mm and clear gap 0.4 mm at least. The printer supplies swatches (at least three golds and two silvers) stamped on the real stock before the order is released."),
        ("Blind emboss", "Depth 0.3 to 0.5 mm, walls 45 to 55 degrees, line width 0.5 mm at least, inner corner radius 0.15 mm at least. The smallest embossed mark is 19.3 mm high: 12 mm fails (0.093 mm), 20 mm passes (0.155 mm). Stocks: black card 400 to 600 g, cotton paper 300 to 600 g; no gloss stock."),
        ("Files", "Curves only; spot colours named FOIL_GOLD, FOIL_SILVER, EMBOSS_DIE; overprint off for foil layers; registration marks supplied by the printer; bleed 3 mm."),
    ]
    cw = 335
    for i, (title, text) in enumerate(cols):
        x = MX + i * (cw + 22)
        card(b, x, 215, cw, 440)
        b.text(x + 22, 255, title, 18, WHITE, 600)
        para(b, x + 22, 290, text, cw - 44, 14)
    para(b, MX, 700, "Open at the first print: the foil type and number are recorded after the first print; Pantone candidates are confirmed against a swatch book; the hot foil needs the horizontal lockup 40.8 mm wide at least.", 1300, 14)


@page(("09", "Applications"), "Application icons", "Tile with the mark; the main drawing from 48 px")
def icons(b):
    items = [("icons/app-icon-graphite.png", "graphite"), ("icons/app-icon-light.png", "light"), ("icons/app-icon-gold.png", "gold"), ("icons/web/icon-512.png", "web 512"), ("icons/web/icon-maskable-512.png", "maskable 512")]
    for i, (rel, name) in enumerate(items):
        x = MX + i * 284
        pic(b, rel, x, 215, 240, 240, radius=36, frame=False)
        label(b, x, 480, name, 13)
    rows = [
        ["Platform", "Files", "Parameters"],
        ["macOS", "nurt.icns, nurt.iconset", "11 entries; tile 824 on the 1024 canvas, squircle with exponent 5, shadow in two layers"],
        ["Windows", "nurt.ico, nurt-tile-150.png, nurt-tile-44.png", "16, 24, 32, 48, 64, 128, 256 px (PNG in ICO); 16, 24 and 32 from the drawings S16, S24, S32; tile 150 has silver on a transparent ground and needs the dark background colour of the manifest"],
        ["Linux", "hicolor/NxN/apps/nurt.png, scalable/apps/nurt.svg, symbolic/apps/nurt-symbolic.svg", "N = 16, 22, 24, 32, 48, 64, 96, 128, 192, 256, 512; symbolic in currentColor"],
        ["Web", "favicon.ico, favicon.svg, apple-touch-icon.png, icon-192.png, icon-512.png, icon-maskable-512.png, site.webmanifest", "favicon.svg uses the drawing S16 without a tile: silver and gold on a dark theme, ink and deep gold on a light one (prefers-color-scheme)"],
    ]
    table(b, MX, 520, [140, 520, 748], rows, size=12.5, first_bold=True)


@page(("09", "Applications"), "Icon scale test", "Silver and gold stay apart from 16 px to 128 px; the Linux icons at real size")
def icon_scale(b):
    sizes = (16, 22, 24, 32, 48, 64, 96, 128)
    for row, ground in enumerate((TOKENS_GRAPHITE, TOKENS_PAPER)):
        y0 = 215 + row * 230
        b.rect(MX, y0, 1408, 210, fill=ground, stroke=LINE, rx=8)
        x = MX + 48
        for n in sizes:
            pic(b, f"icons/linux/hicolor/{n}x{n}/apps/nurt.png", x, y0 + 24 + (128 - n), n, n, radius=0, frame=False)
            label(b, x, y0 + 24 + 128 + 26, f"{n} px", 12, fill=MUTED if row == 0 else "#4A515C")
            x += n + 100
    para(b, MX, 700, "The lightness contrast of silver and gold in grey scale at 16 to 96 px was measured on the delivered pixels of the icons: 1.87 to 1.89 : 1 (tokens: 1.91 : 1). The tile has a gradient from #1A1D22 to #0B0C0E, and the mark fills 58 per cent of it. The icons of 192, 256 and 512 px are made from the same drawing.", 1300, 15)


@page(("09", "Applications"), "Social card and documentation", "Metal lockup on the trace of the stream; the mark in the navigation")
def web_pages(b):
    pic(b, "mockups/mockup-social-card.png", MX, 215, 640, 336, cover=True)
    pic(b, "mockups/mockup-docs-header-dark.png", 760, 215, 744, 186, cover=True)
    pic(b, "mockups/mockup-docs-header-light.png", 760, 421, 744, 186, cover=True)
    label(b, MX, 580, "Social card 1200 x 630: the metal horizontal lockup and the sentence of the product.", 13)
    label(b, 760, 640, "Documentation headers 1920 x 480, dark and light: the mark 30 px high in the navigation, colour variant.", 13)


@page(("09", "Applications"), "Stationery", "Gold foil on black card; blind emboss on paper")
def stationery(b):
    pic(b, "mockups/mockup-business-card.png", MX, 215, 460, 300, cover=True)
    pic(b, "mockups/mockup-letterhead.png", 580, 215, 460, 300, cover=True)
    pic(b, "mockups/mockup-stationery-set.png", 1064, 215, 440, 300, cover=True)
    para(b, MX, 560, "Front of the card: gold foil on black card. Back of the card and the letterhead: blind emboss on paper. Envelope: colour variant. No gloss stock; the minimum sizes of foil and emboss are in the chapter on the mark and in the print specification.", 1300, 15)


# The banner drawings and their roles are copied from /crates/nurt-contracts/src/product.rs of release 0.1.1.
BANNER_BLOCKS = [
    "           ▄██",
    "        ▄▄████",
    "      ▄██████▀",
    "   ▄██████▀",
    " ▄█████▀▀   ▄▄▄",
    "██████▄     ████▄",
    " ▀██████▄   ▀██████▄",
    "   ▀▀██████▄  ▀██████▄▄",
    "      ▀██████▄   ▀██████▄",
    "         ▀████     ▀██████",
    "           ▀▀▀   ▄▄█████▀",
    "               ▄██████▀",
    "            ▄██████▀",
    "            ████▀▀",
    "            ██▀",
]
BANNER_BLOCKS_ROLES = [
    "           sss",
    "        ssssss",
    "      ssssssss",
    "   ssssssss",
    " ssssssss   ggg",
    "sssssss     ggggg",
    " ssssssss   gggggggg",
    "   sssssssss  ggggggggg",
    "      ssssssss   gggggggg",
    "         sssss     ggggggg",
    "           sss   gggggggg",
    "               gggggggg",
    "            gggggggg",
    "            gggggg",
    "            ggg",
]
BANNER_ASCII = [
    "          +#+",
    "       :+###+",
    "    :+######:",
    "  +######+",
    "+#####+:   +#+:",
    "######+    +####+",
    " :+######: :+#####+:",
    "    :+#####+: :######+:",
    "       +####+    +######",
    "         :+#+   :+#####+",
    "              +######+",
    "           :######+:",
    "           +###+:",
    "           +#+",
]
BANNER_ASCII_ROLES = [
    "          sss",
    "       ssssss",
    "    sssssssss",
    "  ssssssss",
    "ssssssss   gggg",
    "sssssss    gggggg",
    " sssssssss ggggggggg",
    "    sssssssss ggggggggg",
    "       ssssss    ggggggg",
    "         ssss   gggggggg",
    "              gggggggg",
    "           ggggggggg",
    "           gggggg",
    "           ggg",
]
BANNER_GAP = 3
TEXT_ROWS = [3, 5, 7, 9, 11]
# The advance of Geist Mono is 0.6 em.
MONO_ADVANCE = 0.6


def banner_text():
    return [("Danaco Nurt", 700, WHITE), (TAGLINE, 400, BODY), ("nurt " + VERSION, 400, BODY), (HINT, 400, BODY), (COPYRIGHT, 400, BODY)]


def draw_banner(b, x, y, cw, ch, rows, roles, blocks):
    """Draw a banner drawing of the product; every cell has the role of the product (s silver, g gold)."""
    over = 0.3
    if blocks:
        for r, row in enumerate(rows):
            for half in (0, 1):
                runs = []
                for c, glyph in enumerate(row):
                    on = glyph == "█" or (glyph == "▀" and half == 0) or (glyph == "▄" and half == 1)
                    colour = None
                    if on:
                        colour = SILVER if roles[r][c] == "s" else GOLD
                    if runs and runs[-1][2] == colour and runs[-1][1] == c:
                        runs[-1][1] = c + 1
                    else:
                        runs.append([c, c + 1, colour])
                for c0, c1, colour in runs:
                    if colour:
                        b.rect(x + c0 * cw - over, y + r * ch + half * ch / 2 - over, (c1 - c0) * cw + 2 * over, ch / 2 + 2 * over, fill=colour)
    else:
        size = cw / MONO_ADVANCE
        for r, row in enumerate(rows):
            for c, glyph in enumerate(row):
                if glyph == " ":
                    continue
                colour = SILVER if roles[r][c] == "s" else GOLD
                b.text(x + c * cw, y + r * ch + ch * 0.75, glyph, size, colour, 500, family="GeistMono")


def terminal_chrome(b, x, y, w, h):
    card(b, x, y, w, h, fill="#08090B")
    for k in range(3):
        b.circle(x + 26 + 22 * k, y + 26, 6, fill="#30343B")


@page(("10", "Terminal"), "The start banner", "The mark in half blocks beside the text of the product")
def banner(b):
    cw, ch = 11, 22
    terminal_chrome(b, MX, 215, 1408, 480)
    ox, oy = MX + 40, 270
    draw_banner(b, ox, oy, cw, ch, BANNER_BLOCKS, BANNER_BLOCKS_ROLES, True)
    tx = ox + (26 + BANNER_GAP) * cw
    size = cw / MONO_ADVANCE
    for row, (line, weight, colour) in zip(TEXT_ROWS, banner_text()):
        b.text(tx, oy + row * ch + ch * 0.7, line, size, colour, weight, family="GeistMono")
    label(b, ox, oy + 15 * ch + 34, "Half blocks: 26 columns x 15 rows (26 x 30 pixels). The left chevron is silver, the right chevron is gold. The text starts three columns after the drawing, on rows 3, 5, 7, 9 and 11.", 13)
    para(b, MX, 730, "The banner is written only to an interactive terminal; scripts, pipes and the protocol server receive nothing. Text lines: the name (bold where styling is enabled), the sentence of the product, the version line, the hint and the copyright line.", 1300, 14.5)


@page(("10", "Terminal"), "Fallback and colours", "7-bit ASCII drawing, palette indices and the typed line")
def banner_fallback(b):
    terminal_chrome(b, MX, 215, 560, 480)
    draw_banner(b, MX + 40, 270, 13, 26, BANNER_ASCII, BANNER_ASCII_ROLES, False)
    label(b, MX + 40, 270 + 14 * 26 + 30, "ASCII: 24 columns x 14 rows, the characters # + : and space.", 13)
    x = 700
    rows = [
        ["Colour", "24-bit", "256 palette", "Role"],
        ["silver", "#DDE1E6  221;225;230", "254", "the left chevron, the indicator of the prompt, the pipe"],
        ["gold", "#C9A15A  201;161;90", "179", "the right chevron, the word nurt of the prompt, commands"],
        ["cyan", "#4FD1F5  79;209;245", "81", "strings of a typed line"],
    ]
    y = table(b, x, 225, [100, 240, 110, 354], rows, size=13, first_bold=True)
    px, py = x, y + 60
    size = 20
    cx = px
    for s, colour, weight in (("nurt", GOLD, 500), ("> ", SILVER, 500), ("find", GOLD, 700), (' "/srv/app"', CYAN, 400), (" |> ", SILVER, 700), ("first 5", BODY, 400)):
        b.text(cx, py, s, size, colour, weight, family="GeistMono")
        cx += BD.text_width(s, size, weight, family="GeistMono")
    label(b, px, py + 34, "A typed line: the commands are gold and bold, the strings cyan, the pipe silver and bold, as shipped in 0.1.1.", 13)
    para(b, x, py + 70, "A terminal that reports 24-bit colour shows exactly these values; any other terminal gets the nearest colours of the 256-colour palette. NO_COLOR or a disabled style turns the colours off and leaves the glyphs.", 804, 14)


@page(("10", "Terminal"), "Selection of the drawing", "By terminal and by width")
def banner_rules(b):
    rows = [
        ["Condition", "Drawing", "Colour"],
        ["UTF-8 terminal (LANG, LC_ALL or LC_CTYPE contains UTF-8 or utf8; on Windows: Windows Terminal or console output code page 65001)", "half blocks", "24-bit when the terminal reports true colour, otherwise palette 254 / 179"],
        ["any other terminal, including the legacy Windows console", "7-bit ASCII", "the same rule"],
        ["styling disabled (NO_COLOR)", "as above", "none; glyphs only"],
    ]
    y = table(b, MX, 215, [700, 220, 488], rows, size=14)
    rows = [
        ["Width of the terminal", "Layout"],
        ["wide enough for the drawing, the gap of 3 columns and the longest text line", "the text stands beside the drawing on rows 3, 5, 7, 9 and 11"],
        ["narrower than that", "the text goes below the drawing after one empty line"],
        ["narrower than the drawing", "the drawing is left out and only the text is written"],
    ]
    y = table(b, MX, y + 40, [700, 708], rows, size=14)
    para(b, MX, y + 36, "Turning each drawing by 180 degrees with the roles exchanged (and, for half blocks, the upper and lower half exchanged) gives the same drawing: each chevron is one connected shape and the chevrons are separated by at least one clear cell in every row where both occur. A test of the product checks it.", 1300, 14.5)


@page(("11", "Rules"), "Rules of use", "Seven rules and the forbidden uses")
def rules(b):
    b.text(MX, 250, "Rules", 20, WHITE, 600)
    bullets(b, MX, 292, 700, [
        "Use the supplied files; do not redraw or retype the mark or the name.",
        "Dark ground: color-dark or white. Light ground: color-light or black. Foil: gold. Screens with large formats: metal.",
        "Clear space: the cap height of the wordmark in lockups, a quarter of the box height around the mark alone.",
        "Below 48 px use the drawings S32, S24 and S16; never scale the main drawing down below 48 px.",
        "The silver chevron is always left and raised, the gold chevron right and lowered; the tones are never swapped.",
        "Gold appears in tint only as the deep gold #A3722A on light grounds, never as text on #F6F7F8.",
        "Cyan is the blue of the terminal, for strings in a typed line, not for the logo.",
    ], size=15, gap=12)
    b.text(860, 250, "Forbidden", 20, WHITE, 600)
    bullets(b, 860, 292, 644, [
        "swapping the tones of the chevrons",
        "stretching or squeezing; changing the angle or the stroke",
        "mirroring: the figure is turned, not reflected",
        "outlining; shadows and glow",
        "colour on a ground that keeps less than 3 : 1",
        "busy or coloured backgrounds",
        "rotation",
        "another typeface for the name",
        "crowding the clear space",
        "gradients that are not in the system",
    ], size=15, gap=10)


@page(("11", "Rules"), "Name and wording", "The sentences that belong to the product")
def wording(b):
    rows = [
        ["Item", "Text"],
        ["Product name", "Danaco Nurt"],
        ["Program name", "nurt"],
        ["Sentence of the product", TAGLINE],
        ["Version line", "nurt " + VERSION],
        ["Hint of the banner", HINT],
        ["Copyright line", COPYRIGHT],
        ["Licence line", "License: Apache-2.0"],
    ]
    y = table(b, MX, 220, [340, 1068], rows, size=15, first_bold=True)
    para(b, MX, y + 40, "In running text write Danaco Nurt for the product and nurt for the program and its commands. The company name Danaco steps back in regular weight; the product name Nurt steps forward in bold.", 1300, 15)
    para(b, MX, y + 100, "The sentence of the product is the same everywhere: the description of the package that builds the nurt executable, the first paragraph of the README and the second line of the banner.", 1300, 15)


@page(("12", "Files"), "Files of the identity", "Where each asset lives")
def files(b):
    rows = [
        ["Path", "Content"],
        ["production/mark/", "mark in 10 variants (SVG, PNG, PDF), drawings 16, 24 and 32 px, construction and small-size sheets"],
        ["production/wordmark/", "wordmark in 9 variants (SVG, PNG, PDF)"],
        ["production/lockups/", "horizontal, vertical and compact in 10 variants (SVG, PNG, PDF)"],
        ["production/backgrounds/", "seven grounds (SVG, PNG), two repeatable tiles, compatibility.json"],
        ["production/render/", "five 3D renders and render-metadata.json"],
        ["production/icons/", "application icons, favicon, macOS, Windows, Linux and web sets, scale test"],
        ["production/print/", "print specification, separations, previews, colour and geometry values"],
        ["production/mockups/", "applications as mockups; the banner as text and as ANSI files"],
        ["production/identity.json, tokens.css", "decided values and the paths of the mark; colour tokens"],
        ["production/production.md", "the specification: decisions, values, rules of use, inventory"],
        ["crates/nurt-cli/src/theme.rs", "silver, gold and cyan of the terminal, with the palette indices 254, 179 and 81"],
        ["crates/nurt-contracts/src/product.rs", "the banner drawings with their roles, the text lines of the banner"],
    ]
    y = table(b, MX, 215, [440, 968], rows, size=13.5, first_bold=True, mono=(0,))
    para(b, MX, y + 28, "The files of the pack are generated by the scripts in production/build and are not edited by hand; this book is generated by build/build_book.py from the same pack and from the values shipped in the product.", 1300, 14)


@page(None)
def closing(b):
    b.mark_drawing(S.MAIN, 700, 230, 300)
    b.text(W / 2, 640, "Danaco Nurt", 40, WHITE, 600, anchor="middle", tracking=-10)
    b.text(W / 2, 684, TAGLINE, 17, SOFT, 400, anchor="middle")
    b.text(W / 2, 760, "https://www.danaco-group.pl  ·  support@danaco-group.pl", 14, MUTED, 400, anchor="middle")
    b.text(W / 2, 788, COPYRIGHT, 13, MUTED, 400, anchor="middle")




def main():
    out = ARGUMENTS.out
    png_dir = ARGUMENTS.png_dir
    out.parent.mkdir(parents=True, exist_ok=True)
    if png_dir is not None:
        png_dir.mkdir(parents=True, exist_ok=True)
    total = len(PAGES)
    work = tempfile.mkdtemp(prefix="nurt-book-")
    files_ = []
    for index, (chapter, title, sub, fn) in enumerate(PAGES, start=1):
        b = BD.Board(W, H, BG)
        fn(b)
        if index not in (1, total):
            frame(b, index, total, chapter, title, sub)
        path = os.path.join(work, f"p{index:02d}.svg")
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(b.svg())
        files_.append(path)
        if png_dir is not None:
            b.save(str(png_dir / f"p{index:02d}.png"))
    env = dict(os.environ, SOURCE_DATE_EPOCH=EPOCH)
    subprocess.run(["rsvg-convert", "-f", "pdf", "-o", str(out), *files_], check=True, env=env)
    for path in files_:
        os.remove(path)
    os.rmdir(work)
    print(f"{out} written ({total} pages)")


if __name__ == "__main__":
    main()

# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# Support: support@danaco-group.pl
