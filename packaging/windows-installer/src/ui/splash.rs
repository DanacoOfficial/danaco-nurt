// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/splash.rs
// Responsibility:   Splash of the wizard: the timeline of section 7.2 of the design, drawn in the window of the wizard.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Splash of the wizard.
//!
//! The splash runs 6.2 seconds in the same window: the trace of the stream fades in, the silver
//! chevron steps back and the gold chevron steps forward, a glint passes over the mark, a terminal
//! opens under it and types `nurt> setup {version}` and `nurt> undo`, the terminal closes, the mark
//! moves into the lockup with the name, and the wizard opens. The times and the coordinates are
//! those of the table of the design (960 x 600 points).

use eframe::egui::{Color32, Mesh, Painter, Pos2, Rect, Shape, Vec2, epaint::Vertex, pos2, vec2};

use super::draw::{Canvas, Run};
use super::mark::{Art, Image, MARK_CENTER};
use super::motion::{in_out_cubic, lerp, norm, out_back, out_cubic, smoothstep};

/// Length of the splash in seconds.
pub(crate) const END: f32 = 6.2;

/// Time to which a key or a click jumps: the finished lockup.
pub(crate) const SKIP_TO: f32 = 5.65;

/// The splash cannot be skipped before this time.
pub(crate) const SKIP_AFTER: f32 = 0.4;

/// Time at which the wizard begins to fade in.
pub(crate) const WIZARD_FROM: f32 = 5.85;

/// Opacity of the wizard under the splash.
pub(crate) fn wizard_alpha(t: f32) -> f32 {
    smoothstep(norm(t, WIZARD_FROM, END))
}

/// Opacity of the splash itself.
pub(crate) fn splash_alpha(t: f32) -> f32 {
    1.0 - smoothstep(norm(t, 5.7, 6.0))
}

/// The lines that the terminal types: the start, the number of characters per second and the segments.
struct Typed {
    start: f32,
    per_second: f32,
    segments: Vec<(&'static str, String)>,
}

fn rows(version: &str) -> Vec<Typed> {
    vec![
        Typed {
            start: 2.4,
            per_second: 38.0,
            segments: vec![
                ("gold", "nurt".into()),
                ("silver", "> ".into()),
                ("command", "setup".into()),
                ("body", format!(" {version}")),
            ],
        },
        Typed {
            start: 3.05,
            per_second: 70.0,
            segments: vec![("soft", "  every change of this setup is journaled".into())],
        },
        Typed {
            start: 3.9,
            per_second: 28.0,
            segments: vec![
                ("gold", "nurt".into()),
                ("silver", "> ".into()),
                ("command", "undo".into()),
            ],
        },
        Typed {
            start: 4.4,
            per_second: 90.0,
            segments: vec![
                ("silver", "  ready".into()),
                ("soft", "  -  nothing has been changed yet".into()),
            ],
        },
    ]
}

fn total_chars(row: &Typed) -> usize {
    row.segments
        .iter()
        .map(|(_, text)| text.chars().count())
        .sum()
}

/// The transform of a group of the splash: `screen = origin + unit * scale + translation`.
#[derive(Clone, Copy)]
struct Place {
    scale: f32,
    translation: Vec2,
}

impl Place {
    fn point(self, origin: Pos2, unit: Vec2) -> Pos2 {
        origin + unit * self.scale + self.translation
    }
}

fn image_rect(origin: Pos2, place: Place, image: &Image, extra: Vec2) -> Rect {
    let min = place.point(origin, image.origin + extra);
    let max = place.point(origin, image.origin + extra + image.size);
    Rect::from_min_max(min, max)
}

fn draw_image(painter: &Painter, image: &Image, rect: Rect, alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    painter.image(
        image.texture.id(),
        rect,
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        Color32::WHITE.gamma_multiply(alpha),
    );
}

/// The glint: a band of white skewed by -36.87 degrees over a mask of a chevron.
fn draw_glint(
    painter: &Painter,
    mask: &Image,
    origin: Pos2,
    place: Place,
    band_center: f32,
    alpha: f32,
) {
    const COLS: usize = 40;
    const ROWS: usize = 40;
    let mut mesh = Mesh::with_texture(mask.texture.id());
    let value = |unit: Vec2| {
        let yg = unit.y - MARK_CENTER.1;
        let xg = unit.x - band_center + 0.75 * yg;
        0.7 * (1.0 - xg.abs() / 60.0).max(0.0) * alpha
    };
    for row in 0..=ROWS {
        for col in 0..=COLS {
            let (u, v) = (col as f32 / COLS as f32, row as f32 / ROWS as f32);
            let unit = mask.origin + vec2(mask.size.x * u, mask.size.y * v);
            let pos = place.point(origin, unit);
            let a = value(unit).clamp(0.0, 1.0);
            mesh.vertices.push(Vertex {
                pos,
                uv: pos2(u, v),
                color: Color32::WHITE.gamma_multiply(a),
            });
        }
    }
    for row in 0..ROWS {
        for col in 0..COLS {
            let i = (row * (COLS + 1) + col) as u32;
            let n = (COLS + 1) as u32;
            mesh.indices
                .extend_from_slice(&[i, i + 1, i + n, i + 1, i + n + 1, i + n]);
        }
    }
    painter.add(Shape::mesh(mesh));
}

/// Draws the trace of the stream: hairlines at the slope 3 : 4 that fade toward the centre, and the pair that drifts.
fn draw_trace(c: &mut Canvas<'_>, t: f32) {
    let opacity = out_cubic(norm(t, 0.0, 0.7));
    if opacity <= 0.0 {
        return;
    }
    let line = Color32::from_rgb(0x23, 0x26, 0x2B);
    let mask = |x: f32, y: f32| {
        let d = (((x - 480.0) / (0.62 * 960.0)).powi(2) + ((y - 276.0) / (0.62 * 600.0)).powi(2))
            .sqrt()
            .min(1.0);
        if d < 0.45 {
            lerp(0.0, 0.12, d / 0.45)
        } else {
            lerp(0.12, 0.85, (d - 0.45) / 0.55)
        }
    };
    // Lines through (19.2 k, 14.4 k) in the direction (-0.6, 0.8), spaced 24 points apart.
    for k in -40..=60 {
        let base = vec2(19.2 * k as f32, 14.4 * k as f32);
        let direction = vec2(-0.6, 0.8);
        let (mut s0, mut s1) = (f32::MIN, f32::MAX);
        for (axis, low, high) in [(0, 0.0, 960.0), (1, 0.0, 600.0)] {
            let (b, d) = if axis == 0 {
                (base.x, direction.x)
            } else {
                (base.y, direction.y)
            };
            let (a, e) = ((low - b) / d, (high - b) / d);
            s0 = s0.max(a.min(e));
            s1 = s1.min(a.max(e));
        }
        if s0 >= s1 {
            continue;
        }
        let steps = (((s1 - s0) / 12.0).ceil() as usize).max(1);
        for step in 0..steps {
            let (a, b) = (
                s0 + (s1 - s0) * step as f32 / steps as f32,
                s0 + (s1 - s0) * (step + 1) as f32 / steps as f32,
            );
            let (p, q) = (base + direction * a, base + direction * b);
            let mid = (p + q) / 2.0;
            let alpha = mask(mid.x, mid.y) * opacity;
            c.painter.line_segment(
                [c.point(p.x, p.y), c.point(q.x, q.y)],
                eframe::egui::Stroke::new(1.0, c.col(line.gamma_multiply(alpha))),
            );
        }
    }
    let drift = t * 10.0;
    let silver = c.pal.silver.gamma_multiply(0.35 * 0.9 * opacity);
    let gold = c.pal.gold.gamma_multiply(0.45 * 0.9 * opacity);
    c.painter.line_segment(
        [
            c.point(-200.0 + drift, -150.0 + drift * 0.75),
            c.point(1160.0 + drift, 870.0 + drift * 0.75),
        ],
        eframe::egui::Stroke::new(1.0, c.col(silver)),
    );
    c.painter.line_segment(
        [
            c.point(-200.0 + drift, -144.0 + drift * 0.75),
            c.point(1160.0 + drift, 876.0 + drift * 0.75),
        ],
        eframe::egui::Stroke::new(1.0, c.col(gold)),
    );
}

/// The terminal under the mark: the frame and the typed text.
fn draw_terminal(c: &mut Canvas<'_>, t: f32, version: &str) {
    let term_in = out_cubic(norm(t, 2.0, 2.45));
    let term_out = norm(t, 4.75, 5.15);
    let opacity = term_in * (1.0 - term_out);
    if opacity <= 0.0 {
        return;
    }
    let shift = 14.0 * (1.0 - term_in) + 10.0 * term_out;
    let pal = c.pal;
    let saved = c.dy;
    c.dy += shift;
    let saved_alpha = c.alpha;
    c.alpha *= opacity;
    c.painter.rect_filled(
        c.rect(190.0, 372.0, 580.0, 164.0),
        0.0,
        c.col(Color32::from_rgb(0x1F, 0x22, 0x26)),
    );
    c.painter.rect_filled(
        c.rect(191.0, 373.0, 578.0, 162.0),
        0.0,
        c.col(Color32::from_rgb(0x0E, 0x10, 0x13)),
    );
    let baselines = [410.0, 438.0, 474.0, 502.0];
    let rows = rows(version);
    let (mut active, mut active_count, mut typing) = (0usize, 0usize, false);
    for (index, row) in rows.iter().enumerate() {
        let typed = if t < row.start {
            0
        } else {
            (((t - row.start) * row.per_second).floor() as usize).min(total_chars(row))
        };
        let mut left = typed;
        let mut runs: Vec<Run> = Vec::new();
        for (kind, text) in &row.segments {
            let take = text.chars().count().min(left);
            left -= take;
            if take == 0 {
                continue;
            }
            let slice: String = text.chars().take(take).collect();
            let (color, weight) = match *kind {
                "gold" => (pal.gold, 400.0),
                "silver" => (pal.silver, 400.0),
                "command" => (pal.gold, 700.0),
                "body" => (pal.text_body, 400.0),
                _ => (pal.text_soft, 400.0),
            };
            runs.push(Run::mono(slice, 15.0, weight, color));
        }
        if !runs.is_empty() {
            c.text(214.0, baselines[index] - 15.0, f32::INFINITY, &runs, 19.0);
        }
        if typed > 0 {
            active = index;
            active_count = typed;
            typing = typed < total_chars(row);
        }
    }
    let blink = ((t * 2.2).floor() as i64) % 2 == 0;
    if t > 2.45 && t < 4.85 && (typing || blink) {
        c.painter.rect_filled(
            c.rect(
                214.0 + 9.0 * active_count as f32,
                baselines[active] - 14.0,
                9.0,
                19.0,
            ),
            0.0,
            c.col(pal.gold),
        );
    }
    c.alpha = saved_alpha;
    c.dy = saved;
}

/// Draws the splash at the time `t`; the canvas carries the fade of the splash.
pub(crate) fn draw(c: &mut Canvas<'_>, art: &Art, t: f32, version: &str) {
    c.painter.rect_filled(
        Rect::from_min_size(c.origin, vec2(960.0, 600.0)),
        0.0,
        c.col(c.pal.graphite),
    );
    draw_trace(c, t);

    // The mark: the chevrons step toward each other, then the whole mark moves into the lockup.
    let p = out_back(norm(t, 0.35, 1.65));
    let opacity = ((t - 0.35) / 0.6).clamp(0.0, 1.0);
    let offset = (1.0 - p) * 46.0;
    let m = in_out_cubic(norm(t, 4.9, 5.5));
    let final_translation = vec2(480.0 - 1088.836 * 0.5 / 2.0, 300.0 - MARK_CENTER.1 * 0.5);
    let scale = lerp(1.0, 0.5, m);
    let cx = lerp(480.0, final_translation.x + MARK_CENTER.0 * 0.5, m);
    let cy = lerp(196.0, final_translation.y + MARK_CENTER.1 * 0.5, m);
    let place = Place {
        scale,
        translation: vec2(cx - MARK_CENTER.0 * scale, cy - MARK_CENTER.1 * scale),
    };
    let origin = c.origin;
    draw_image(
        &c.painter,
        &art.metal_back,
        image_rect(origin, place, &art.metal_back, vec2(offset, 0.0)),
        opacity * c.alpha,
    );
    draw_image(
        &c.painter,
        &art.metal_forward,
        image_rect(origin, place, &art.metal_forward, vec2(-offset, 0.0)),
        opacity * c.alpha,
    );
    let g = norm(t, 1.7, 2.3);
    if g > 0.0 && g < 1.0 {
        let band = lerp(-60.0, 260.0, g);
        draw_glint(&c.painter, &art.mask_back, origin, place, band, c.alpha);
        draw_glint(&c.painter, &art.mask_forward, origin, place, band, c.alpha);
    }

    // The name appears beside the mark.
    let name_place = Place {
        scale: 0.5,
        translation: final_translation,
    };
    let n1 = out_cubic(norm(t, 5.0, 5.5));
    let n2 = out_cubic(norm(t, 5.15, 5.65));
    draw_image(
        &c.painter,
        &art.name_light,
        image_rect(
            origin,
            name_place,
            &art.name_light,
            vec2(-18.0 * (1.0 - n1), 0.0),
        ),
        n1 * c.alpha,
    );
    draw_image(
        &c.painter,
        &art.name_strong,
        image_rect(
            origin,
            name_place,
            &art.name_strong,
            vec2(-18.0 * (1.0 - n2), 0.0),
        ),
        n2 * c.alpha,
    );

    draw_terminal(c, t, version);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
