// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/draw.rs
// Responsibility:   Drawing context of the wizard: placement in the 960 x 600 window, fade and offset of a screen, and the text of the design.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Drawing context of the wizard.
//!
//! The wizard is drawn at the fixed size of the design, 960 x 600 points. A [`Canvas`] maps the
//! coordinates of the design to the screen, applies the fade and the offset of a screen that is
//! entering or leaving, and lays out the text with the faces and the sizes of section 5.

use std::sync::Arc;

use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, Align2, Color32, Galley, Painter, Pos2, Rect, Response, Sense, TextFormat, Ui, Vec2, vec2,
};

use super::theme::{self, Palette};

/// A run of text with its style.
#[derive(Debug, Clone)]
pub(crate) struct Run {
    /// The text.
    pub(crate) text: String,
    /// Size in points.
    pub(crate) size: f32,
    /// Weight of the face, 100 to 900.
    pub(crate) weight: f32,
    /// Colour before the fade.
    pub(crate) color: Color32,
    /// The monospaced face.
    pub(crate) mono: bool,
    /// Extra space between letters, in points.
    pub(crate) spacing: f32,
}

impl Run {
    /// A run of the sans face.
    pub(crate) fn sans(text: impl Into<String>, size: f32, weight: f32, color: Color32) -> Self {
        Self {
            text: text.into(),
            size,
            weight,
            color,
            mono: false,
            spacing: 0.0,
        }
    }

    /// A run of the monospaced face.
    pub(crate) fn mono(text: impl Into<String>, size: f32, weight: f32, color: Color32) -> Self {
        Self {
            text: text.into(),
            size,
            weight,
            color,
            mono: true,
            spacing: 0.0,
        }
    }

    /// Sets the spacing of the letters, in thousandths of the size.
    pub(crate) fn tracking(mut self, per_mille: f32) -> Self {
        self.spacing = self.size * per_mille / 1000.0;
        self
    }
}

/// The drawing context of one screen.
pub(crate) struct Canvas<'a> {
    /// The interface.
    pub(crate) ui: &'a mut Ui,
    /// Painter that is clipped to the window.
    pub(crate) painter: Painter,
    /// Top-left corner of the window on the screen.
    pub(crate) origin: Pos2,
    /// Colours.
    pub(crate) pal: Palette,
    /// Fade of the screen, 0 to 1.
    pub(crate) alpha: f32,
    /// Offset of the screen in y, in points.
    pub(crate) dy: f32,
    /// The ring of the focus is drawn: the keyboard drives the interface.
    pub(crate) ring: bool,
    /// The screen takes the pointer and the keyboard.
    pub(crate) live: bool,
}

impl<'a> Canvas<'a> {
    /// Maps a rectangle of the design to the screen.
    pub(crate) fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.origin + vec2(x, y + self.dy), vec2(w, h))
    }

    /// Maps a point of the design to the screen.
    pub(crate) fn point(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y + self.dy)
    }

    /// The colour after the fade.
    pub(crate) fn col(&self, color: Color32) -> Color32 {
        if self.alpha >= 0.999 {
            color
        } else {
            color.gamma_multiply(self.alpha)
        }
    }

    /// Makes the layout of runs for a width; `line_height` is the height of a line.
    pub(crate) fn job(&self, runs: &[Run], width: f32, line_height: f32) -> LayoutJob {
        let mut job = LayoutJob::default();
        for run in runs {
            let font = if run.mono {
                theme::mono(run.size)
            } else {
                theme::sans(run.size)
            };
            job.append(
                &run.text,
                0.0,
                TextFormat {
                    font_id: font,
                    color: self.col(run.color),
                    line_height: Some(line_height),
                    extra_letter_spacing: run.spacing,
                    coords: theme::weight(run.weight),
                    valign: egui::Align::Center,
                    ..TextFormat::default()
                },
            );
        }
        job.wrap.max_width = width;
        job.wrap.break_anywhere = runs.iter().all(|run| run.mono);
        job
    }

    /// The galley of runs.
    pub(crate) fn galley(&self, runs: &[Run], width: f32, line_height: f32) -> Arc<Galley> {
        let job = self.job(runs, width, line_height);
        self.painter.layout_job(job)
    }

    /// Height of runs wrapped at a width.
    pub(crate) fn measure(&self, runs: &[Run], width: f32, line_height: f32) -> Vec2 {
        self.galley(runs, width, line_height).size()
    }

    /// Draws runs as a paragraph at a point of the design; returns the height. The text is a label of the
    /// accessibility tree, so that a reader of the screen can read it.
    pub(crate) fn text(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        runs: &[Run],
        line_height: f32,
    ) -> f32 {
        let galley = self.galley(runs, width, line_height);
        let size = galley.size();
        if self.live {
            let rect = Rect::from_min_size(
                self.point(x, y),
                vec2(width.max(size.x), size.y.max(line_height)),
            );
            let label = runs.iter().map(|run| run.text.as_str()).collect::<String>();
            let response = self.ui.interact(
                rect,
                self.ui.scope_id().with((
                    "text",
                    label.clone(),
                    (x * 8.0) as i32,
                    (y * 8.0) as i32,
                )),
                Sense::hover(),
            );
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label.clone()));
        }
        self.painter
            .galley(self.point(x, y), galley, Color32::WHITE);
        size.y
    }

    /// Draws one line of text aligned to a point; returns the width.
    pub(crate) fn text_at(
        &mut self,
        anchor: Align2,
        x: f32,
        y: f32,
        runs: &[Run],
        line_height: f32,
    ) -> f32 {
        let galley = self.galley(runs, f32::INFINITY, line_height);
        let size = galley.size();
        let rect = anchor.anchor_size(self.point(x, y), size);
        self.painter.galley(rect.min, galley, Color32::WHITE);
        size.x
    }

    /// Interaction area of a widget with its accessibility description.
    pub(crate) fn interact(
        &mut self,
        id: impl std::hash::Hash + std::fmt::Debug,
        rect: Rect,
        sense: Sense,
    ) -> Response {
        let id = self.ui.scope_id().with(id);
        if self.live {
            self.ui.interact(rect, id, sense)
        } else {
            self.ui.interact(rect, id, Sense::hover())
        }
    }

    /// The focus ring of a control: 2 points of gold at 3 points of distance.
    pub(crate) fn focus_ring(&self, rect: Rect, distance: f32, cut: Option<(f32, f32)>) {
        let ring = rect.expand(distance + 1.0);
        let color = self.col(self.pal.gold);
        match cut {
            Some((x, y)) => {
                let points = super::shape::cut_polygon(ring, x, y);
                super::shape::polyline(&self.painter, &points, 2.0, color, true);
            }
            None => {
                self.painter.rect_stroke(
                    ring,
                    0.0,
                    egui::Stroke::new(2.0, color),
                    egui::StrokeKind::Middle,
                );
            }
        }
    }

    /// Animated value that eases to `target` in `seconds`.
    pub(crate) fn ease(
        &self,
        id: impl std::hash::Hash + std::fmt::Debug,
        target: bool,
        seconds: f32,
    ) -> f32 {
        self.ui.ctx().animate_bool_with_time(
            self.ui.scope_id().with(("ease", id)),
            target,
            if self.pal.high_contrast { 0.0 } else { seconds },
        )
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
