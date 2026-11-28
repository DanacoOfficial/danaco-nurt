// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/mark.rs
// Responsibility:   Art of the wizard: the mark and the name of Danaco Nurt, rasterised from the paths of the identity into textures.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Art of the wizard.
//!
//! The paths come from the files of the identity that the repository holds
//! (`branding/svg/lockup-horizontal-color-dark.svg` and `branding/identity.json`); nothing
//! is redrawn. A path is flattened to polygons and rasterised with an anti-aliasing of 4 x
//! the rows and the exact coverage of the columns, so that the mark is sharp at the scale of
//! the display. The textures are made once per display scale.

use eframe::egui::{Color32, ColorImage, Context, TextureHandle, TextureOptions, Vec2, vec2};

/// Lockup of the identity for dark grounds: the mark (silver and gold) and the name.
static LOCKUP: &str = include_str!("../../../../branding/svg/lockup-horizontal-color-dark.svg");

/// Identity: the paths of the mark on the grid of 96.
static IDENTITY: &str = include_str!("../../../../branding/identity.json");

/// Box of the lockup in its own units: `viewBox="0 -157 1088.836 220"`.
pub(crate) const LOCKUP_BOX: (f32, f32, f32, f32) = (0.0, -157.0, 1088.836, 220.0);

/// Centre of the mark in the units of the lockup.
pub(crate) const MARK_CENTER: (f32, f32) = (96.4, -47.0);

/// A closed polygon.
type Poly = Vec<[f32; 2]>;

/// A texture with its place: `origin` is the corner of its box in the units of the source and `size`
/// is the size of the box in the units of the source.
#[derive(Clone)]
pub(crate) struct Image {
    /// The texture.
    pub(crate) texture: TextureHandle,
    /// Corner of the box in the units of the source.
    pub(crate) origin: Vec2,
    /// Size of the box in the units of the source.
    pub(crate) size: Vec2,
}

/// The textures of the wizard.
pub(crate) struct Art {
    /// Display scale that the textures were made for.
    pub(crate) scale: f32,
    /// The lockup of the rail, flat tones; the box is the viewBox of the lockup.
    pub(crate) rail_lockup: Image,
    /// The mark at 56 points on the grid of 96, flat tones (Finish and Removed).
    pub(crate) seal: Image,
    /// Silver chevron of the splash, metal, at the scale 1 of the lockup.
    pub(crate) metal_back: Image,
    /// Gold chevron of the splash, metal, at the scale 1 of the lockup.
    pub(crate) metal_forward: Image,
    /// White coverage of the silver chevron, for the glint.
    pub(crate) mask_back: Image,
    /// White coverage of the gold chevron, for the glint.
    pub(crate) mask_forward: Image,
    /// The word Danaco, flat tone, at the scale 0.5 of the lockup.
    pub(crate) name_light: Image,
    /// The word Nurt, flat tone, at the scale 0.5 of the lockup.
    pub(crate) name_strong: Image,
}

// ------------------------------------------------------------------------------------------
// Parsing and flattening of paths

fn numbers_of(text: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let start = index;
        if matches!(bytes[index], b'-' | b'+') {
            index += 1;
        }
        let mut seen_dot = false;
        let mut digits = 0;
        while index < bytes.len()
            && (bytes[index].is_ascii_digit() || (bytes[index] == b'.' && !seen_dot))
        {
            seen_dot |= bytes[index] == b'.';
            digits += usize::from(bytes[index].is_ascii_digit());
            index += 1;
        }
        if digits > 0 && index < bytes.len() && matches!(bytes[index], b'e' | b'E') {
            index += 1;
            if index < bytes.len() && matches!(bytes[index], b'-' | b'+') {
                index += 1;
            }
            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }
        }
        if digits > 0 {
            if let Ok(value) = text[start..index].parse::<f32>() {
                out.push(value);
            }
        } else {
            index = start + 1;
        }
    }
    out
}

fn cubic(out: &mut Poly, p: [[f32; 2]; 4]) {
    for step in 1..=16 {
        let t = step as f32 / 16.0;
        let u = 1.0 - t;
        let x = u * u * u * p[0][0]
            + 3.0 * u * u * t * p[1][0]
            + 3.0 * u * t * t * p[2][0]
            + t * t * t * p[3][0];
        let y = u * u * u * p[0][1]
            + 3.0 * u * u * t * p[1][1]
            + 3.0 * u * t * t * p[2][1]
            + t * t * t * p[3][1];
        out.push([x, y]);
    }
}

fn quadratic(out: &mut Poly, p: [[f32; 2]; 3]) {
    for step in 1..=12 {
        let t = step as f32 / 12.0;
        let u = 1.0 - t;
        out.push([
            u * u * p[0][0] + 2.0 * u * t * p[1][0] + t * t * p[2][0],
            u * u * p[0][1] + 2.0 * u * t * p[1][1] + t * t * p[2][1],
        ]);
    }
}

/// Parses an SVG path of the commands M, L, H, V, C, Q and Z (absolute) into closed polygons.
pub(crate) fn parse_path(d: &str) -> Vec<Poly> {
    let mut polygons: Vec<Poly> = Vec::new();
    let mut current: Poly = Vec::new();
    let mut at = [0.0f32, 0.0];
    let mut command = ' ';
    let mut arguments = String::new();
    let flush = |command: char,
                 arguments: &str,
                 current: &mut Poly,
                 at: &mut [f32; 2],
                 polygons: &mut Vec<Poly>| {
        let n = numbers_of(arguments);
        match command {
            'M' => {
                if current.len() > 2 {
                    polygons.push(std::mem::take(current));
                }
                current.clear();
                if n.len() >= 2 {
                    *at = [n[0], n[1]];
                    current.push(*at);
                }
            }
            'L' => {
                for pair in n.as_chunks::<2>().0 {
                    *at = [pair[0], pair[1]];
                    current.push(*at);
                }
            }
            'H' => {
                for value in &n {
                    at[0] = *value;
                    current.push(*at);
                }
            }
            'V' => {
                for value in &n {
                    at[1] = *value;
                    current.push(*at);
                }
            }
            'C' => {
                for six in n.as_chunks::<6>().0 {
                    cubic(
                        current,
                        [*at, [six[0], six[1]], [six[2], six[3]], [six[4], six[5]]],
                    );
                    *at = [six[4], six[5]];
                }
            }
            'Q' => {
                for four in n.as_chunks::<4>().0 {
                    quadratic(current, [*at, [four[0], four[1]], [four[2], four[3]]]);
                    *at = [four[2], four[3]];
                }
            }
            'Z' | 'z' => {
                if current.len() > 2 {
                    *at = current[0];
                    polygons.push(std::mem::take(current));
                }
                current.clear();
            }
            _ => {}
        }
    };
    for character in d.chars() {
        if character.is_ascii_alphabetic() && character != 'e' && character != 'E' {
            flush(command, &arguments, &mut current, &mut at, &mut polygons);
            command = character;
            arguments.clear();
            if character == 'Z' || character == 'z' {
                flush('Z', "", &mut current, &mut at, &mut polygons);
                command = ' ';
            }
        } else {
            arguments.push(character);
        }
    }
    flush(command, &arguments, &mut current, &mut at, &mut polygons);
    if current.len() > 2 {
        polygons.push(current);
    }
    polygons
}

/// The `d` of every `<path fill="…" d="…">` of an SVG text, with its fill.
fn paths_of(svg: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = svg;
    while let Some(start) = rest.find("<path ") {
        let tag_end = rest[start..]
            .find('>')
            .map_or(rest.len(), |end| start + end);
        let tag = &rest[start..tag_end];
        let attribute = |name: &str| {
            let key = format!("{name}=\"");
            tag.find(&key).and_then(|at| {
                let from = at + key.len();
                tag[from..]
                    .find('"')
                    .map(|to| tag[from..from + to].to_string())
            })
        };
        if let (Some(fill), Some(d)) = (attribute("fill"), attribute("d")) {
            out.push((fill, d));
        }
        rest = &rest[tag_end.min(rest.len())..];
        if rest.is_empty() {
            break;
        }
        rest = &rest[1..];
    }
    out
}

fn bounds(polygons: &[Poly]) -> [f32; 4] {
    let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for point in polygons.iter().flatten() {
        bounds[0] = bounds[0].min(point[0]);
        bounds[1] = bounds[1].min(point[1]);
        bounds[2] = bounds[2].max(point[0]);
        bounds[3] = bounds[3].max(point[1]);
    }
    bounds
}

// ------------------------------------------------------------------------------------------
// Rasterisation

/// Coverage of a set of polygons, filled with the non-zero rule; `min` is the corner of the raster in the
/// units of the polygons and `scale` is the number of pixels of a unit.
pub(crate) fn coverage(
    polygons: &[Poly],
    min: [f32; 2],
    scale: f32,
    width: usize,
    height: usize,
) -> Vec<f32> {
    const ROWS: usize = 4;
    let mut edges: Vec<(f32, f32, f32, f32, i32)> = Vec::new();
    for polygon in polygons {
        for index in 0..polygon.len() {
            let a = polygon[index];
            let b = polygon[(index + 1) % polygon.len()];
            let (x0, y0, x1, y1) = (
                (a[0] - min[0]) * scale,
                (a[1] - min[1]) * scale,
                (b[0] - min[0]) * scale,
                (b[1] - min[1]) * scale,
            );
            if (y0 - y1).abs() < 1e-6 {
                continue;
            }
            if y0 < y1 {
                edges.push((x0, y0, x1, y1, 1))
            } else {
                edges.push((x1, y1, x0, y0, -1))
            }
        }
    }
    let mut cover = vec![0f32; width * height];
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for row in 0..height {
        for sub in 0..ROWS {
            let y = row as f32 + (sub as f32 + 0.5) / ROWS as f32;
            crossings.clear();
            for &(x0, y0, x1, y1, direction) in &edges {
                if y0 <= y && y < y1 {
                    crossings.push((x0 + (y - y0) / (y1 - y0) * (x1 - x0), direction));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for index in 0..crossings.len() {
                winding += crossings[index].1;
                if winding != 0 && index + 1 < crossings.len() {
                    add_span(
                        &mut cover[row * width..(row + 1) * width],
                        crossings[index].0,
                        crossings[index + 1].0,
                        1.0 / ROWS as f32,
                    );
                }
            }
        }
    }
    cover
}

fn add_span(row: &mut [f32], x0: f32, x1: f32, weight: f32) {
    let width = row.len() as f32;
    let (x0, x1) = (x0.clamp(0.0, width), x1.clamp(0.0, width));
    if x1 <= x0 {
        return;
    }
    let first = x0.floor() as usize;
    let last = (x1.ceil() as usize).min(row.len());
    for (index, cell) in row.iter_mut().enumerate().take(last).skip(first) {
        let overlap = (x1.min(index as f32 + 1.0) - x0.max(index as f32)).max(0.0);
        *cell += overlap * weight;
    }
}

fn texture(
    context: &Context,
    name: &str,
    width: usize,
    height: usize,
    pixel: impl Fn(usize, usize) -> Color32,
) -> TextureHandle {
    let mut bytes = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let color = pixel(x, y);
            bytes.extend_from_slice(&[color.r(), color.g(), color.b(), color.a()]);
        }
    }
    context.load_texture(
        name,
        ColorImage::from_rgba_premultiplied([width, height], &bytes),
        TextureOptions::LINEAR,
    )
}

fn premultiplied(color: [u8; 3], cover: f32) -> Color32 {
    let a = cover.clamp(0.0, 1.0);
    Color32::from_rgba_premultiplied(
        (color[0] as f32 * a).round() as u8,
        (color[1] as f32 * a).round() as u8,
        (color[2] as f32 * a).round() as u8,
        (a * 255.0).round() as u8,
    )
}

fn hex(text: &str) -> [u8; 3] {
    let value = u32::from_str_radix(text.trim_start_matches('#'), 16).unwrap_or(0);
    [(value >> 16) as u8, (value >> 8) as u8, value as u8]
}

type Stops = &'static [(f32, [u8; 3])];

const SILVER_METAL: Stops = &[
    (0.0, [0xF7, 0xF9, 0xFB]),
    (0.16, [0xC3, 0xC9, 0xD1]),
    (0.36, [0xEE, 0xF1, 0xF4]),
    (0.58, [0x9A, 0xA1, 0xAA]),
    (0.8, [0xD8, 0xDC, 0xE2]),
    (1.0, [0x8A, 0x91, 0x9A]),
];

const GOLD_METAL: Stops = &[
    (0.0, [0xF6, 0xE3, 0xB4]),
    (0.22, [0xD6, 0xAE, 0x66]),
    (0.46, [0xB9, 0x89, 0x3F]),
    (0.7, [0xF0, 0xD6, 0xA0]),
    (1.0, [0xA9, 0x79, 0x2F]),
];

fn stop_color(stops: Stops, t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    for pair in stops.windows(2) {
        if t <= pair[1].0 {
            let k = (t - pair[0].0) / (pair[1].0 - pair[0].0).max(1e-6);
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
            return [
                mix(pair[0].1[0], pair[1].1[0]),
                mix(pair[0].1[1], pair[1].1[1]),
                mix(pair[0].1[2], pair[1].1[2]),
            ];
        }
    }
    stops[stops.len() - 1].1
}

/// Colour of the metal gradient at a point of the box of the shape: the gradient of the identity, from
/// (0.05, 0) to (0.95, 1) in the units of the box.
fn metal_at(stops: Stops, u: f32, v: f32) -> [u8; 3] {
    let (dx, dy) = (0.9, 1.0);
    let t = ((u - 0.05) * dx + v * dy) / (dx * dx + dy * dy);
    stop_color(stops, t)
}

impl Art {
    /// Makes the textures for a display scale (pixels per point).
    pub(crate) fn new(context: &Context, scale: f32) -> Self {
        let scale = scale.max(1.0);
        let paths = paths_of(LOCKUP);
        let polygons: Vec<Vec<Poly>> = paths.iter().map(|(_, d)| parse_path(d)).collect();
        let fill: Vec<[u8; 3]> = paths.iter().map(|(color, _)| hex(color)).collect();

        // The rail: the whole lockup, flat tones, at 150 points of width.
        let rail_scale = 150.0 / LOCKUP_BOX.2 * scale;
        let (rw, rh) = (
            (LOCKUP_BOX.2 * rail_scale).ceil() as usize,
            (LOCKUP_BOX.3 * rail_scale).ceil() as usize,
        );
        let covers: Vec<Vec<f32>> = polygons
            .iter()
            .map(|polygon| coverage(polygon, [LOCKUP_BOX.0, LOCKUP_BOX.1], rail_scale, rw, rh))
            .collect();
        let rail = texture(context, "nurt-rail-lockup", rw, rh, |x, y| {
            let mut out = Color32::TRANSPARENT;
            for (index, cover) in covers.iter().enumerate() {
                let c = cover[y * rw + x];
                if c > 0.0 {
                    out = blend(out, premultiplied(fill[index], c));
                }
            }
            out
        });
        let rail_lockup = Image {
            texture: rail,
            origin: vec2(LOCKUP_BOX.0, LOCKUP_BOX.1),
            size: vec2(LOCKUP_BOX.2, LOCKUP_BOX.3),
        };

        // The splash: the two chevrons with the metal gradients at the scale 1 of the lockup.
        let metal = |index: usize, stops: Stops, name: &str, mask: bool| {
            let b = bounds(&polygons[index]);
            let (x0, y0) = (b[0].floor(), b[1].floor());
            let (w, h) = (
                ((b[2].ceil() - x0) * scale) as usize + 1,
                ((b[3].ceil() - y0) * scale) as usize + 1,
            );
            let cover = coverage(&polygons[index], [x0, y0], scale, w, h);
            let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
            let tex = texture(context, name, w, h, |x, y| {
                let c = cover[y * w + x];
                if mask {
                    return premultiplied([255, 255, 255], c);
                }
                let u = ((x as f32 + 0.5) / scale + x0 - b[0]) / bw;
                let v = ((y as f32 + 0.5) / scale + y0 - b[1]) / bh;
                premultiplied(metal_at(stops, u, v), c)
            });
            Image {
                texture: tex,
                origin: vec2(x0, y0),
                size: vec2(w as f32 / scale, h as f32 / scale),
            }
        };
        let metal_back = metal(0, SILVER_METAL, "nurt-metal-back", false);
        let metal_forward = metal(1, GOLD_METAL, "nurt-metal-forward", false);
        let mask_back = metal(0, SILVER_METAL, "nurt-mask-back", true);
        let mask_forward = metal(1, GOLD_METAL, "nurt-mask-forward", true);

        // The name at the scale 0.5 of the lockup.
        let name = |index: usize, label: &str| {
            let b = bounds(&polygons[index]);
            let s = 0.5 * scale;
            let (x0, y0) = (b[0].floor() - 1.0, b[1].floor() - 1.0);
            let (w, h) = (
                ((b[2].ceil() + 1.0 - x0) * s) as usize + 2,
                ((b[3].ceil() + 1.0 - y0) * s) as usize + 2,
            );
            let cover = coverage(&polygons[index], [x0, y0], s, w, h);
            let tex = texture(context, label, w, h, |x, y| {
                premultiplied(fill[index], cover[y * w + x])
            });
            Image {
                texture: tex,
                origin: vec2(x0, y0),
                size: vec2(w as f32 / s, h as f32 / s),
            }
        };
        let name_light = name(2, "nurt-name-light");
        let name_strong = name(3, "nurt-name-strong");

        // The seal: the mark on the grid of 96, at 56 points.
        let (back, forward) = seal_paths();
        let seal_polygons = [parse_path(&back), parse_path(&forward)];
        let seal_scale = 56.0 / 96.0 * scale;
        let side = (56.0 * scale).ceil() as usize;
        let seal_covers: Vec<Vec<f32>> = seal_polygons
            .iter()
            .map(|polygon| coverage(polygon, [0.0, 0.0], seal_scale, side, side))
            .collect();
        let seal_colors = [hex("#DDE1E6"), hex("#C9A15A")];
        let seal_tex = texture(context, "nurt-seal", side, side, |x, y| {
            let mut out = Color32::TRANSPARENT;
            for (index, cover) in seal_covers.iter().enumerate() {
                let c = cover[y * side + x];
                if c > 0.0 {
                    out = blend(out, premultiplied(seal_colors[index], c));
                }
            }
            out
        });
        let seal = Image {
            texture: seal_tex,
            origin: vec2(0.0, 0.0),
            size: vec2(96.0, 96.0),
        };

        Self {
            scale,
            rail_lockup,
            seal,
            metal_back,
            metal_forward,
            mask_back,
            mask_forward,
            name_light,
            name_strong,
        }
    }
}

/// Source over of two premultiplied colours.
fn blend(below: Color32, above: Color32) -> Color32 {
    let inverse = 255 - above.a() as u32;
    let mix = |b: u8, a: u8| (a as u32 + b as u32 * inverse / 255).min(255) as u8;
    Color32::from_rgba_premultiplied(
        mix(below.r(), above.r()),
        mix(below.g(), above.g()),
        mix(below.b(), above.b()),
        mix(below.a(), above.a()),
    )
}

fn seal_paths() -> (String, String) {
    let value: serde_json::Value = serde_json::from_str(IDENTITY).unwrap_or_default();
    let take = |key: &str| {
        value["mark"]["paths_main"][key]
            .as_str()
            .unwrap_or("")
            .to_string()
    };
    (take("left"), take("right"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lockup_has_four_paths_and_the_mark_is_two_chevrons() {
        let paths = paths_of(LOCKUP);
        assert_eq!(paths.len(), 4);
        assert_eq!(paths[0].0, "#DDE1E6");
        assert_eq!(paths[1].0, "#C9A15A");
        let (back, forward) = seal_paths();
        assert!(!parse_path(&back).is_empty() && !parse_path(&forward).is_empty());
    }

    #[test]
    fn a_square_covers_its_area() {
        let square = vec![vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]]];
        let cover = coverage(&square, [0.0, 0.0], 1.0, 4, 4);
        assert!(cover.iter().all(|c| (c - 1.0).abs() < 1e-3));
        let half = coverage(&square, [-0.5, 0.0], 1.0, 5, 4);
        assert!((half[0] - 0.5).abs() < 1e-3 && (half[1] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_hole_is_not_filled() {
        let outer = vec![[0.0, 0.0], [6.0, 0.0], [6.0, 6.0], [0.0, 6.0]];
        let hole = vec![[2.0, 2.0], [2.0, 4.0], [4.0, 4.0], [4.0, 2.0]];
        let cover = coverage(&[outer, hole], [0.0, 0.0], 1.0, 6, 6);
        assert!(cover[3 * 6 + 3].abs() < 1e-3 && (cover[0] - 1.0).abs() < 1e-3);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
