// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/shape.rs
// Responsibility:   Shapes of the design: the cut at the slope 3 : 4, the chevron, the tick and the shield, drawn as polygons and strokes.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Shapes of the design.
//!
//! Buttons, cards, choices and dialogs are cut at the top right and at the bottom left, at
//! the slope 3 : 4 (section 3 of the design). The chevron is the only marker of a choice, a
//! list item and progress. All shapes are made of polygons and polylines, so that they are
//! sharp at every scale of the display.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, epaint::PathStroke, pos2};

/// Points of a rectangle cut at the top right and at the bottom left.
pub(crate) fn cut_polygon(rect: Rect, cut_x: f32, cut_y: f32) -> Vec<Pos2> {
    vec![
        pos2(rect.left(), rect.top()),
        pos2(rect.right() - cut_x, rect.top()),
        pos2(rect.right(), rect.top() + cut_y),
        pos2(rect.right(), rect.bottom()),
        pos2(rect.left() + cut_x, rect.bottom()),
        pos2(rect.left(), rect.bottom() - cut_y),
    ]
}

/// Clips a polygon with a convex polygon (Sutherland-Hodgman); the clip must be convex and clockwise on the screen.
pub(crate) fn clip_convex(subject: &[Pos2], clip: &[Pos2]) -> Vec<Pos2> {
    let mut output: Vec<Pos2> = subject.to_vec();
    for index in 0..clip.len() {
        if output.is_empty() {
            break;
        }
        let a = clip[index];
        let b = clip[(index + 1) % clip.len()];
        let inside = |p: Pos2| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x) >= -1e-4;
        let input = std::mem::take(&mut output);
        for current in 0..input.len() {
            let p = input[current];
            let q = input[(current + 1) % input.len()];
            match (inside(p), inside(q)) {
                (true, true) => output.push(q),
                (true, false) => output.push(intersection(p, q, a, b)),
                (false, true) => {
                    output.push(intersection(p, q, a, b));
                    output.push(q);
                }
                (false, false) => {}
            }
        }
    }
    output
}

fn intersection(p: Pos2, q: Pos2, a: Pos2, b: Pos2) -> Pos2 {
    let (dx1, dy1) = (q.x - p.x, q.y - p.y);
    let (dx2, dy2) = (b.x - a.x, b.y - a.y);
    let denominator = dx1 * dy2 - dy1 * dx2;
    if denominator.abs() < 1e-9 {
        return q;
    }
    let t = ((a.x - p.x) * dy2 - (a.y - p.y) * dx2) / denominator;
    pos2(p.x + t * dx1, p.y + t * dy1)
}

/// Fills a polygon that is convex.
pub(crate) fn fill(painter: &Painter, points: Vec<Pos2>, color: Color32) {
    if points.len() >= 3 && color.a() > 0 {
        painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    }
}

/// Fills a cut rectangle.
pub(crate) fn fill_cut(painter: &Painter, rect: Rect, cut_x: f32, cut_y: f32, color: Color32) {
    fill(painter, cut_polygon(rect, cut_x, cut_y), color);
}

/// A cut rectangle with a border of one point that follows the diagonal: the outer layer carries the
/// colour of the border, the inner layer is inset by `border` and cut by the same slope.
pub(crate) fn bordered_cut(
    painter: &Painter,
    rect: Rect,
    cut_x: f32,
    cut_y: f32,
    border: Color32,
    inner: Color32,
    width: f32,
) {
    fill_cut(painter, rect, cut_x, cut_y, border);
    let inset = rect.shrink(width);
    fill_cut(painter, inset, cut_x - width, cut_y - width, inner);
}

/// A rectangle with a border of one point and no cut.
pub(crate) fn bordered_rect(
    painter: &Painter,
    rect: Rect,
    border: Color32,
    inner: Color32,
    width: f32,
) {
    painter.rect_filled(rect, 0.0, border);
    painter.rect_filled(rect.shrink(width), 0.0, inner);
}

/// A polyline with a stroke, and a round join at every inner point as the design draws the chevron.
pub(crate) fn polyline(
    painter: &Painter,
    points: &[Pos2],
    width: f32,
    color: Color32,
    closed: bool,
) {
    if points.len() < 2 || color.a() == 0 {
        return;
    }
    let stroke = PathStroke::new(width, color);
    painter.add(Shape::Path(eframe::egui::epaint::PathShape {
        points: points.to_vec(),
        closed,
        fill: Color32::TRANSPARENT,
        stroke,
    }));
    let radius = width / 2.0;
    let joins: Vec<Pos2> = if closed {
        points.to_vec()
    } else {
        points[1..points.len() - 1].to_vec()
    };
    for point in joins {
        painter.circle_filled(point, radius, color);
    }
}

/// The chevron of the marker: `M2 2L12 9.5L2 17` in a box of 18 x 22 points at `origin`.
pub(crate) fn chevron_marker(painter: &Painter, origin: Pos2, color: Color32) {
    let points = [
        origin + eframe::egui::vec2(3.0, 2.0),
        origin + eframe::egui::vec2(15.0, 11.0),
        origin + eframe::egui::vec2(3.0, 20.0),
    ];
    polyline(painter, &points, 3.4, color, false);
}

/// The chevron of the progress: `M2 2L12 9.5L2 17`, shifted by 1.5, in a box of 14 x 22 points at `origin`.
pub(crate) fn chevron_progress(painter: &Painter, origin: Pos2, color: Color32) {
    let points = [
        origin + eframe::egui::vec2(2.0, 3.5),
        origin + eframe::egui::vec2(12.0, 11.0),
        origin + eframe::egui::vec2(2.0, 18.5),
    ];
    polyline(painter, &points, 3.2, color, false);
}

/// The tick of a check: `M2 6.5l2.8 2.8L10 3.5` in a box of 12 x 12 points at `origin`.
pub(crate) fn tick(painter: &Painter, origin: Pos2, color: Color32) {
    let points = [
        origin + eframe::egui::vec2(2.0, 6.5),
        origin + eframe::egui::vec2(4.8, 9.3),
        origin + eframe::egui::vec2(10.0, 3.5),
    ];
    painter.add(Shape::line(points.to_vec(), Stroke::new(2.2, color)));
}

/// The shield glyph of a button that needs an administrator, in a box of 14 x 16 points at `origin`.
pub(crate) fn shield(painter: &Painter, origin: Pos2, color: Color32) {
    let mut points = vec![
        origin + eframe::egui::vec2(7.0, 1.0),
        origin + eframe::egui::vec2(12.5, 3.0),
        origin + eframe::egui::vec2(12.5, 7.5),
    ];
    cubic(
        &mut points,
        origin,
        [(12.5, 7.5), (12.5, 10.7), (10.2, 13.5), (7.0, 15.0)],
    );
    cubic(
        &mut points,
        origin,
        [(7.0, 15.0), (3.8, 13.5), (1.5, 10.7), (1.5, 7.5)],
    );
    points.push(origin + eframe::egui::vec2(1.5, 3.0));
    polyline(painter, &points, 1.6, color, true);
}

fn cubic(points: &mut Vec<Pos2>, origin: Pos2, c: [(f32, f32); 4]) {
    for step in 1..=12 {
        let t = step as f32 / 12.0;
        let u = 1.0 - t;
        let x = u * u * u * c[0].0
            + 3.0 * u * u * t * c[1].0
            + 3.0 * u * t * t * c[2].0
            + t * t * t * c[3].0;
        let y = u * u * u * c[0].1
            + 3.0 * u * u * t * c[1].1
            + 3.0 * u * t * t * c[2].1
            + t * t * t * c[3].1;
        points.push(origin + eframe::egui::vec2(x, y));
    }
}

/// The glyph of a window button: a bar (minimise) or a cross (close) in a box of 12 x 12 points at `origin`.
pub(crate) fn caption_glyph(painter: &Painter, origin: Pos2, close: bool, color: Color32) {
    let stroke = Stroke::new(1.4, color);
    if close {
        painter.line_segment(
            [
                origin + eframe::egui::vec2(1.5, 1.5),
                origin + eframe::egui::vec2(10.5, 10.5),
            ],
            stroke,
        );
        painter.line_segment(
            [
                origin + eframe::egui::vec2(10.5, 1.5),
                origin + eframe::egui::vec2(1.5, 10.5),
            ],
            stroke,
        );
    } else {
        painter.line_segment(
            [
                origin + eframe::egui::vec2(1.0, 6.0),
                origin + eframe::egui::vec2(11.0, 6.0),
            ],
            stroke,
        );
    }
}

/// A rectangle given by its corners.
pub(crate) fn rect_xywh(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(pos2(x, y), eframe::egui::vec2(w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cut_follows_the_slope_three_to_four() {
        let points = cut_polygon(rect_xywh(0.0, 0.0, 100.0, 40.0), 12.0, 9.0);
        assert_eq!(points.len(), 6);
        assert_eq!(points[1], pos2(88.0, 0.0));
        assert_eq!(points[2], pos2(100.0, 9.0));
        assert_eq!(points[4], pos2(12.0, 40.0));
        assert_eq!(points[5], pos2(0.0, 31.0));
    }

    #[test]
    fn clipping_keeps_the_inside_of_the_cut() {
        let clip = cut_polygon(rect_xywh(0.0, 0.0, 12.0, 12.0), 4.0, 3.0);
        let inner = vec![
            pos2(2.0, 2.0),
            pos2(10.0, 2.0),
            pos2(10.0, 10.0),
            pos2(2.0, 10.0),
        ];
        let clipped = clip_convex(&inner, &clip);
        assert!(clipped.len() >= 4);
        assert!(
            clipped
                .iter()
                .all(|p| p.x >= 1.99 && p.x <= 10.01 && p.y >= 1.99 && p.y <= 10.01)
        );
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
