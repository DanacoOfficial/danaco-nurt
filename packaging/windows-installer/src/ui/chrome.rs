// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/chrome.rs
// Responsibility:   Frame of the wizard: the rail with the steps, the caption bar, the footer with the actions, the dialog and the scrollbar.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Frame of the wizard.
//!
//! The rail is a stream (section 3 of the design): the steps hang on one band of equal width,
//! passed steps are silver, the step in progress is gold, the steps ahead are quiet. The caption
//! bar replaces the one of the system; it drags the window and holds Minimize and Close.

use eframe::egui::{self, Color32, CursorIcon, Rect, Sense, ViewportCommand, pos2, vec2};

use super::draw::{Canvas, Run};
use super::mark::LOCKUP_BOX;
use super::shape::{self, bordered_cut, clip_convex, cut_polygon, fill, rect_xywh};
use super::theme::{
    CAPTION_H, CUT_X, CUT_Y, FOOTER_H, MARGIN, MOTION_BASE, RAIL_W, WINDOW_H, WINDOW_W,
};
use super::widgets::{self, ButtonKind};
use super::{Action, App, Dialog, DialogKind};

/// The line of a step of the rail.
const STEP_H: f32 = 40.0;

impl App {
    /// The rail: the lockup, the steps, the sentence about the journal and the version line.
    pub(crate) fn draw_rail(&mut self, c: &mut Canvas<'_>) {
        let pal = c.pal;
        c.painter.rect_filled(
            c.rect(0.0, 0.0, RAIL_W, WINDOW_H),
            0.0,
            c.col(pal.graphite_soft),
        );
        c.painter.rect_filled(
            c.rect(RAIL_W - 1.0, 0.0, 1.0, WINDOW_H),
            0.0,
            c.col(pal.line),
        );
        self.draw_rail_trace(c);
        if let Some(art) = &self.art {
            let size = vec2(150.0, LOCKUP_BOX.3 / LOCKUP_BOX.2 * 150.0);
            c.painter.image(
                art.rail_lockup.texture.id(),
                Rect::from_min_size(c.point(28.0, 28.0), size),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                c.col(Color32::WHITE),
            );
            if c.live {
                let response = c.ui.interact(
                    Rect::from_min_size(c.point(28.0, 28.0), size),
                    c.ui.scope_id().with("brand"),
                    Sense::hover(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Image, true, "Danaco Nurt")
                });
            }
        }
        let steps = self.steps();
        let current = self.index;
        let top = 106.0;
        for (index, label) in steps.iter().enumerate() {
            let y = top + STEP_H * index as f32;
            let state = if index < current {
                0
            } else if index == current {
                1
            } else {
                2
            };
            let center = y + STEP_H / 2.0;
            if index + 1 < steps.len() {
                let band = if index < current {
                    pal.silver
                } else {
                    pal.line
                };
                c.painter
                    .rect_filled(c.rect(28.0 + 5.0, center, 2.0, STEP_H), 0.0, c.col(band));
            }
            let (border, inner) = match state {
                0 => (pal.silver, pal.silver),
                1 => (pal.gold, pal.gold),
                _ => (pal.control_edge, pal.graphite_soft),
            };
            let node = c.rect(28.0, center - 6.0, 12.0, 12.0);
            let outline = cut_polygon(node, 4.0, 3.0);
            fill(&c.painter, outline.clone(), c.col(border));
            let inset = vec![
                node.shrink(2.0).left_top(),
                node.shrink(2.0).right_top(),
                node.shrink(2.0).right_bottom(),
                node.shrink(2.0).left_bottom(),
            ];
            fill(&c.painter, clip_convex(&inset, &outline), c.col(inner));
            let (color, weight) = match state {
                0 => (pal.text_soft, 400.0),
                1 => (pal.text_strong, 600.0),
                _ => (pal.text_muted, 400.0),
            };
            let runs = [Run::sans(label.as_str(), 14.0, weight, color)];
            let height = c.measure(&runs, 180.0, 20.0).y;
            c.text(28.0 + 32.0, center - height / 2.0, 180.0, &runs, 20.0);
            if c.live {
                let rect = c.rect(28.0, y, RAIL_W - 56.0, STEP_H);
                let response =
                    c.ui.interact(rect, c.ui.scope_id().with(("step", index)), Sense::hover());
                let name = format!("{label}, step {} of {}", index + 1, steps.len());
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Label, true, name.clone())
                });
                if state == 1 {
                    c.ui.ctx()
                        .accesskit_node_builder(response.id, |node| node.set_selected(true));
                }
            }
        }
        let note = [Run::sans(
            "Every change this setup makes is written to a journal, so it can be undone.",
            13.0,
            400.0,
            pal.text_soft,
        )];
        let meta = [
            Run::mono(
                format!("nurt {}", crate::app::VERSION),
                12.0,
                400.0,
                pal.text_muted,
            ),
            Run::mono("\n", 12.0, 400.0, pal.text_muted),
            Run::mono(
                format!("Windows {}", crate::requirements::setup_arch()),
                12.0,
                400.0,
                pal.text_muted,
            ),
        ];
        let meta_height = 36.0;
        let note_height = c.measure(&note, RAIL_W - 56.0, 19.0).y;
        let meta_top = WINDOW_H - 24.0 - meta_height;
        c.text(
            28.0,
            meta_top - 14.0 - note_height,
            RAIL_W - 56.0,
            &note,
            19.0,
        );
        c.text(28.0, meta_top, RAIL_W - 56.0, &meta, 18.0);
    }

    /// The hairlines of the trace of the stream in the rail: they fade in from the top to the foot.
    fn draw_rail_trace(&self, c: &mut Canvas<'_>) {
        let line = Color32::from_rgb(0x1D, 0x20, 0x24);
        if c.pal.high_contrast {
            return;
        }
        for k in -20..=30 {
            let base = vec2(19.2 * k as f32, 14.4 * k as f32);
            let direction = vec2(-0.6, 0.8);
            let (mut s0, mut s1) = (f32::MIN, f32::MAX);
            for (low, high, b, d) in [
                (0.0, RAIL_W, base.x, direction.x),
                (0.0, WINDOW_H, base.y, direction.y),
            ] {
                let (a, e) = ((low - b) / d, (high - b) / d);
                s0 = s0.max(a.min(e));
                s1 = s1.min(a.max(e));
            }
            if s0 >= s1 {
                continue;
            }
            let steps = (((s1 - s0) / 20.0).ceil() as usize).max(1);
            for step in 0..steps {
                let (a, b) = (
                    s0 + (s1 - s0) * step as f32 / steps as f32,
                    s0 + (s1 - s0) * (step + 1) as f32 / steps as f32,
                );
                let (p, q) = (base + direction * a, base + direction * b);
                let alpha = (p.y + q.y) / 2.0 / WINDOW_H * 0.9 * 0.9;
                c.painter.line_segment(
                    [c.point(p.x, p.y), c.point(q.x, q.y)],
                    egui::Stroke::new(1.0, c.col(line.gamma_multiply(alpha))),
                );
            }
        }
    }

    /// The caption bar: the area that drags the window and the buttons Minimize and Close.
    pub(crate) fn draw_caption(&mut self, c: &mut Canvas<'_>) {
        let pal = c.pal;
        let drag = c.rect(RAIL_W, 0.0, WINDOW_W - RAIL_W - 92.0, CAPTION_H);
        let response = c.interact("caption-drag", drag, Sense::click_and_drag());
        if c.live && response.drag_started_by(egui::PointerButton::Primary) {
            c.ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
        }
        for (index, (name, close)) in [("Minimize", false), ("Close setup", true)]
            .into_iter()
            .enumerate()
        {
            let x = WINDOW_W - 92.0 + 46.0 * index as f32;
            let rect = c.rect(x, 6.0, 46.0, 32.0);
            let response = c.interact(name, rect, Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
            let hover = c.ease((name, "hover"), response.hovered(), 0.12);
            if hover > 0.0 {
                c.painter
                    .rect_filled(rect, 0.0, c.col(pal.line.gamma_multiply(hover)));
            }
            let glyph = if hover > 0.5 {
                pal.text_strong
            } else {
                pal.text_soft
            };
            shape::caption_glyph(
                &c.painter,
                rect.center() - vec2(6.0, 6.0),
                close,
                c.col(glyph),
            );
            if c.live && c.ring && response.has_focus() {
                c.painter.rect_stroke(
                    rect.shrink(2.0),
                    0.0,
                    egui::Stroke::new(2.0, c.col(pal.gold)),
                    egui::StrokeKind::Inside,
                );
            }
            if c.live && response.clicked() {
                self.pending = Some(if close {
                    Action::Close
                } else {
                    Action::Minimize
                });
            }
        }
    }

    /// The footer: Cancel at the left, then the secondary and the primary action at the right.
    pub(crate) fn draw_footer(&mut self, c: &mut Canvas<'_>) {
        let pal = c.pal;
        let y = WINDOW_H - FOOTER_H;
        c.painter.rect_filled(
            c.rect(RAIL_W, y, WINDOW_W - RAIL_W, 1.0),
            0.0,
            c.col(pal.line),
        );
        let config = self.footer();
        let left = RAIL_W + MARGIN;
        let right = WINDOW_W - MARGIN;
        let top = y + (FOOTER_H - 40.0) / 2.0;
        if let Some(cancel) = &config.cancel {
            let width = widgets::button_width(c, cancel, ButtonKind::Ghost, false);
            if widgets::button(
                c,
                "footer-cancel",
                c.rect(left, top, width, 40.0),
                cancel,
                ButtonKind::Ghost,
                false,
                true,
            ) {
                self.pending = Some(Action::Cancel);
            }
        }
        let mut edge = right;
        if let Some((label, enabled, shield)) = &config.primary {
            let width = widgets::button_width(c, label, ButtonKind::Primary, *shield);
            edge -= width;
            if widgets::button(
                c,
                "footer-primary",
                c.rect(edge, top, width, 40.0),
                label,
                ButtonKind::Primary,
                *shield,
                *enabled,
            ) {
                self.pending = Some(Action::Primary);
            }
            edge -= 12.0;
        }
        if let Some((label, ghost)) = &config.secondary {
            let kind = if *ghost {
                ButtonKind::Ghost
            } else {
                ButtonKind::Secondary
            };
            let width = widgets::button_width(c, label, kind, false);
            edge -= width;
            if widgets::button(
                c,
                "footer-secondary",
                c.rect(edge, top, width, 40.0),
                label,
                kind,
                false,
                true,
            ) {
                self.pending = Some(Action::Secondary);
            }
        }
    }

    /// The scrollbar of a screen whose content is longer than the area: 10 points of `control-edge`.
    pub(crate) fn draw_scrollbar(&mut self, c: &mut Canvas<'_>, content_bottom: f32) {
        let view_top = CAPTION_H;
        let view_h = WINDOW_H - FOOTER_H - CAPTION_H;
        let content_h = (content_bottom + 16.0 - view_top).max(view_h);
        self.max_scroll = (content_h - view_h).max(0.0);
        self.scroll = self.scroll.clamp(0.0, self.max_scroll);
        if self.max_scroll <= 0.0 {
            return;
        }
        let track = c.rect(WINDOW_W - MARGIN - 10.0, view_top, 10.0, view_h);
        let thumb_h = (view_h * view_h / content_h).max(32.0);
        let thumb_y = view_top + (view_h - thumb_h) * self.scroll / self.max_scroll;
        let thumb =
            Rect::from_min_size(pos2(track.min.x, c.origin.y + thumb_y), vec2(10.0, thumb_h));
        let response = c.ui.interact(
            thumb.expand2(vec2(4.0, 0.0)),
            c.ui.scope_id().with("scroll-thumb"),
            Sense::drag(),
        );
        if c.live && response.dragged() {
            let delta = response.drag_delta().y;
            self.scroll = (self.scroll + delta * (self.max_scroll / (view_h - thumb_h).max(1.0)))
                .clamp(0.0, self.max_scroll);
        }
        if c.live && response.hovered() {
            c.ui.ctx().set_cursor_icon(CursorIcon::Default);
        }
        c.painter.rect_filled(thumb, 0.0, c.col(c.pal.control_edge));
    }

    /// The dialog over a scrim: 440 points wide, gold border, the cut of the design.
    pub(crate) fn draw_dialog(&mut self, c: &mut Canvas<'_>, dialog: &Dialog) {
        let pal = c.pal;
        let opened = dialog.opened.elapsed().as_secs_f32();
        let k = if pal.high_contrast || !self.startup.animations {
            1.0
        } else {
            super::motion::norm(opened, 0.0, MOTION_BASE)
        };
        let eased = super::motion::interface(k);
        c.painter.rect_filled(
            c.rect(0.0, 0.0, WINDOW_W, WINDOW_H),
            0.0,
            pal.black_deep.gamma_multiply(0.72 * eased),
        );
        let (title, text, safe, danger) = dialog_texts(dialog.kind);
        let body_runs = [Run::sans(text, 15.0, 400.0, pal.text_soft)];
        let text_height = c.measure(&body_runs, 440.0 - 56.0, 22.0).y;
        let height = 26.0 + 26.0 + 8.0 + text_height + 22.0 + 40.0 + 22.0;
        let x = (WINDOW_W - 440.0) / 2.0;
        let y = (WINDOW_H - height) / 2.0 + 8.0 * (1.0 - eased);
        c.alpha *= eased;
        let rect = c.rect(x, y, 440.0, height);
        bordered_cut(
            &c.painter,
            rect,
            CUT_X,
            CUT_Y,
            c.col(pal.gold),
            c.col(pal.graphite_soft),
            1.0,
        );
        c.text(
            x + 28.0,
            y + 26.0,
            440.0 - 56.0,
            &[Run::sans(title, 20.0, 600.0, pal.text_strong)],
            26.0,
        );
        c.text(
            x + 28.0,
            y + 26.0 + 26.0 + 8.0,
            440.0 - 56.0,
            &body_runs,
            22.0,
        );
        let top = y + height - 22.0 - 40.0;
        let primary_width = widgets::button_width(c, danger, ButtonKind::Primary, false);
        let safe_width = widgets::button_width(c, safe, ButtonKind::Secondary, false);
        let primary_x = x + 440.0 - 28.0 - primary_width;
        let safe_x = primary_x - 12.0 - safe_width;
        if self.dialog_focus_pending {
            c.ui.ctx()
                .memory_mut(|memory| memory.request_focus(c.ui.scope_id().with("dialog-safe")));
            self.dialog_focus_pending = false;
        }
        if widgets::button(
            c,
            "dialog-safe",
            c.rect(safe_x, top, safe_width, 40.0),
            safe,
            ButtonKind::Secondary,
            false,
            true,
        ) {
            self.pending = Some(Action::DialogSafe);
        }
        if widgets::button(
            c,
            "dialog-danger",
            c.rect(primary_x, top, primary_width, 40.0),
            danger,
            ButtonKind::Primary,
            false,
            true,
        ) {
            self.pending = Some(Action::DialogConfirm);
        }
    }
}

/// Title, text, the safe action and the other action of a dialog (section 8.10 of the design).
pub(crate) fn dialog_texts(
    kind: DialogKind,
) -> (&'static str, &'static str, &'static str, &'static str) {
    match kind {
        DialogKind::CancelInstall => (
            "Cancel the setup?",
            "The changes made so far will be undone.",
            "Continue setup",
            "Cancel and undo",
        ),
        DialogKind::CancelUndo => (
            "Cancel the setup?",
            "The entries already reversed stay undone; run the setup again to finish.",
            "Continue setup",
            "Stop",
        ),
        DialogKind::Close => (
            "Close the setup?",
            "Nothing has been changed on this PC.",
            "Stay",
            "Close",
        ),
        DialogKind::ConfirmUndo => (
            "Undo this setup?",
            "The program, the shortcuts and the other changes that this setup made are removed.",
            "Keep Danaco Nurt",
            "Undo setup",
        ),
    }
}

/// A rectangle of the body area of a screen in the design.
pub(crate) fn body_rect() -> Rect {
    rect_xywh(
        RAIL_W,
        CAPTION_H,
        WINDOW_W - RAIL_W,
        WINDOW_H - FOOTER_H - CAPTION_H,
    )
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
