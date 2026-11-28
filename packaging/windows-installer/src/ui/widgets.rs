// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/widgets.rs
// Responsibility:   Components of the design: buttons, cards, choices, checks, the field of a folder, the progress of chevrons, the log and the chips.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Components of the design (section 6 of `design/installer/installer.md`).
//!
//! Every component is cut at the slope 3 : 4 where the design cuts it, uses only the tokens of
//! the palette and describes itself to the tree of UI Automation (role, name, state), so that a
//! reader of the screen announces the same things that the eye reads.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Rect, Role, Sense, TextEdit, WidgetInfo, pos2, vec2,
};

use super::draw::{Canvas, Run};
use super::shape::{self, bordered_cut, bordered_rect, fill_cut};
use super::theme::{self, CUT_X, CUT_Y, MOTION_FAST};
use crate::model::{LogLine, Tone};

/// Kind of a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    /// The default action: gold.
    Primary,
    /// An edge of `control-edge` on graphite.
    Secondary,
    /// No border, soft text.
    Ghost,
}

/// Width that a button needs for its label.
pub(crate) fn button_width(c: &Canvas<'_>, label: &str, kind: ButtonKind, shield: bool) -> f32 {
    let runs = [Run::sans(label, 14.0, 600.0, c.pal.text_strong).tracking(5.0)];
    let text = c.measure(&runs, f32::INFINITY, 20.0).x;
    let padding = if kind == ButtonKind::Ghost {
        28.0
    } else {
        44.0
    };
    let glyph = if shield { 14.0 + 8.0 } else { 0.0 };
    let width = text + glyph + padding;
    if kind == ButtonKind::Ghost {
        width
    } else {
        width.max(104.0)
    }
}

/// A button of the footer or of a dialog; returns that it was pressed.
pub(crate) fn button(
    c: &mut Canvas<'_>,
    id: &str,
    rect: Rect,
    label: &str,
    kind: ButtonKind,
    shield: bool,
    enabled: bool,
) -> bool {
    let response = c.interact(
        id,
        rect,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if enabled && c.live && response.hovered() {
        c.ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, label));
    let hover = c.ease((id, "hover"), enabled && response.hovered(), MOTION_FAST);
    let pressed = enabled && response.is_pointer_button_down_on();
    let shifted = if pressed {
        rect.translate(vec2(0.0, 1.0))
    } else {
        rect
    };
    let pal = c.pal;
    let faded = if enabled { 1.0 } else { 0.4 };
    let alpha = c.alpha;
    let col = move |color: Color32| {
        if alpha >= 0.999 {
            color.gamma_multiply(faded)
        } else {
            color.gamma_multiply(alpha * faded)
        }
    };
    let mix = |from: Color32, to: Color32| {
        Color32::from_rgb(
            lerp_u8(from.r(), to.r(), hover),
            lerp_u8(from.g(), to.g(), hover),
            lerp_u8(from.b(), to.b(), hover),
        )
    };
    let label_color = match kind {
        ButtonKind::Primary => {
            fill_cut(
                &c.painter,
                shifted,
                CUT_X,
                CUT_Y,
                col(mix(pal.gold, pal.gold_hover)),
            );
            pal.on_gold
        }
        ButtonKind::Secondary => {
            bordered_cut(
                &c.painter,
                shifted,
                CUT_X,
                CUT_Y,
                col(pal.control_edge),
                col(mix(pal.graphite, pal.graphite_soft)),
                1.0,
            );
            pal.text_strong
        }
        ButtonKind::Ghost => {
            c.painter.rect_filled(
                shifted,
                0.0,
                col(mix(pal.graphite, pal.graphite_soft)).gamma_multiply(hover),
            );
            mix(pal.text_soft, pal.text_strong)
        }
    };
    let runs = [Run::sans(label, 14.0, 600.0, label_color).tracking(5.0)];
    let galley = c.galley(&runs, f32::INFINITY, 20.0);
    let glyph = if shield { 22.0 } else { 0.0 };
    let total = galley.size().x + glyph;
    let start = shifted.center().x - total / 2.0;
    if shield {
        shape::shield(
            &c.painter,
            pos2(start, shifted.center().y - 8.0),
            col(label_color),
        );
    }
    c.painter.galley(
        pos2(start + glyph, shifted.center().y - galley.size().y / 2.0),
        galley,
        Color32::WHITE,
    );
    if c.live && c.ring && response.has_focus() {
        c.focus_ring(
            shifted,
            3.0,
            if kind == ButtonKind::Ghost {
                None
            } else {
                Some((CUT_X, CUT_Y))
            },
        );
    }
    enabled && c.live && response.clicked()
}

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t.clamp(0.0, 1.0)).round() as u8
}

/// A card: a surface of `graphite-soft` with a border of `line` that follows the cut.
pub(crate) fn card(c: &Canvas<'_>, rect: Rect) {
    bordered_cut(
        &c.painter,
        rect,
        CUT_X,
        CUT_Y,
        c.col(c.pal.line),
        c.col(c.pal.graphite_soft),
        1.0,
    );
}

/// A choice (a radio button drawn as a card); returns that it was pressed.
pub(crate) fn choice(
    c: &mut Canvas<'_>,
    id: &str,
    rect: Rect,
    title: &str,
    text: &str,
    tag: &str,
    selected: bool,
    enabled: bool,
) -> Response2 {
    let response = c.interact(
        id,
        rect,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if enabled && c.live && response.hovered() {
        c.ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response.widget_info(|| {
        WidgetInfo::selected(
            Role::RadioButton,
            enabled,
            selected,
            format!("{title}. {text}"),
        )
    });
    let hover = c.ease((id, "hover"), enabled && response.hovered(), MOTION_FAST);
    let pal = c.pal;
    let faded = if enabled { 1.0 } else { 0.45 };
    let alpha = c.alpha;
    let col = move |color: Color32| {
        if alpha >= 0.999 {
            color.gamma_multiply(faded)
        } else {
            color.gamma_multiply(alpha * faded)
        }
    };
    let border = if selected { pal.gold } else { pal.control_edge };
    let inner = Color32::from_rgb(
        lerp_u8(pal.graphite_soft.r(), pal.choice_hover.r(), hover),
        lerp_u8(pal.graphite_soft.g(), pal.choice_hover.g(), hover),
        lerp_u8(pal.graphite_soft.b(), pal.choice_hover.b(), hover),
    );
    bordered_cut(&c.painter, rect, CUT_X, CUT_Y, col(border), col(inner), 1.0);
    shape::chevron_marker(&c.painter, rect.min + vec2(17.0, 15.0), col(border));
    let width = rect.width() - 17.0 - 18.0 - 14.0 - 16.0 - 8.0;
    let tag_width = if tag.is_empty() {
        0.0
    } else {
        c.measure(
            &[Run::mono(tag, 11.0, 400.0, pal.text_muted)],
            f32::INFINITY,
            16.0,
        )
        .x + 12.0
    };
    let text_x = rect.min.x - c.origin.x + 17.0 + 18.0 + 14.0;
    let top = rect.min.y - c.origin.y - c.dy;
    let title_runs = [Run::sans(title, 15.0, 600.0, pal.text_strong)];
    let body_runs = [Run::sans(text, 13.0, 400.0, pal.text_soft)];
    let mut dummy = 0.0;
    c.text_dim(
        text_x,
        top + 15.0,
        width - tag_width,
        &title_runs,
        22.0,
        faded,
        &mut dummy,
    );
    c.text_dim(
        text_x,
        top + 15.0 + 22.0 + 2.0,
        width - tag_width,
        &body_runs,
        19.0,
        faded,
        &mut dummy,
    );
    if !tag.is_empty() {
        c.text_at(
            Align2::RIGHT_TOP,
            rect.max.x - c.origin.x - 16.0 - 1.0,
            top + 15.0 + 3.0,
            &[Run::mono(tag, 11.0, 400.0, pal.text_muted)],
            16.0,
        );
    }
    if c.live && c.ring && response.has_focus() {
        c.focus_ring(rect, 3.0, Some((CUT_X, CUT_Y)));
    }
    Response2 {
        clicked: enabled && c.live && response.clicked(),
        focused: response.has_focus(),
    }
}

/// Result of a choice.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Response2 {
    /// The choice was pressed.
    pub(crate) clicked: bool,
    /// The choice has the keyboard focus.
    pub(crate) focused: bool,
}

impl Canvas<'_> {
    /// Draws a paragraph that is dimmed by `factor`; returns the height in `height`.
    pub(crate) fn text_dim(
        &mut self,
        x: f32,
        y: f32,
        width: f32,
        runs: &[Run],
        line_height: f32,
        factor: f32,
        height: &mut f32,
    ) {
        if (factor - 1.0).abs() < 1e-3 {
            *height = self.text(x, y, width, runs, line_height);
            return;
        }
        let dimmed: Vec<Run> = runs
            .iter()
            .map(|run| Run {
                color: run.color.gamma_multiply(factor),
                ..run.clone()
            })
            .collect();
        *height = self.text(x, y, width, &dimmed, line_height);
    }
}

/// A check; the box is at `(x, y)` of the design and the text goes to its right. Returns the height of the
/// row and whether it was pressed.
#[allow(
    clippy::too_many_arguments,
    reason = "the position, the label and the state of a check are separate values of the design"
)]
pub(crate) fn check(
    c: &mut Canvas<'_>,
    id: &str,
    x: f32,
    y: f32,
    width: f32,
    title: &str,
    new: bool,
    text: &str,
    checked: bool,
    enabled: bool,
) -> (f32, bool) {
    let pal = c.pal;
    let text_width = width - 32.0;
    let title_runs = [Run::sans(title, 15.0, 400.0, pal.text_strong)];
    let body_runs = [Run::sans(text, 13.0, 400.0, pal.text_muted)];
    let title_height = c.measure(&title_runs, text_width, 22.0).y;
    let body_height = if text.is_empty() {
        0.0
    } else {
        c.measure(&body_runs, text_width, 19.0).y
    };
    let height = 20.0 + title_height + body_height;
    let rect = c.rect(x, y, width, height);
    let response = c.interact(
        id,
        rect,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if enabled && c.live && response.hovered() {
        c.ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response.widget_info(|| {
        WidgetInfo::selected(
            Role::CheckBox,
            enabled,
            checked,
            if text.is_empty() {
                title.to_string()
            } else {
                format!("{title}. {text}")
            },
        )
    });
    let faded = if enabled { 1.0 } else { 0.55 };
    let alpha = c.alpha;
    let col = move |color: Color32| {
        if alpha >= 0.999 {
            color.gamma_multiply(faded)
        } else {
            color.gamma_multiply(alpha * faded)
        }
    };
    let box_rect = c.rect(x, y + 11.0, 20.0, 20.0);
    let fill = if checked { pal.gold } else { pal.control_edge };
    fill_cut(&c.painter, box_rect, 6.0, 4.5, col(fill));
    fill_cut(
        &c.painter,
        box_rect.shrink(1.0),
        5.0,
        3.75,
        col(if checked { pal.gold } else { pal.graphite }),
    );
    if checked {
        shape::tick(&c.painter, box_rect.min + vec2(4.0, 4.0), col(pal.on_gold));
    }
    let mut used = 0.0;
    c.text_dim(
        x + 32.0,
        y + 10.0,
        text_width,
        &title_runs,
        22.0,
        faded,
        &mut used,
    );
    if new {
        let new_runs = [Run::mono("NEW", 10.0, 400.0, pal.gold).tracking(60.0)];
        let title_w = c.measure(&title_runs, f32::INFINITY, 22.0).x;
        let tag_w = c.measure(&new_runs, f32::INFINITY, 14.0).x + 12.0;
        let tag = c.rect(x + 32.0 + title_w + 8.0, y + 10.0 + 4.0, tag_w, 14.0);
        bordered_rect(
            &c.painter,
            tag,
            col(pal.gold_deep),
            col(pal.graphite_soft),
            1.0,
        );
        c.text_at(
            Align2::CENTER_CENTER,
            tag.center().x - c.origin.x,
            tag.center().y - c.origin.y - c.dy,
            &new_runs.map(|run| Run {
                color: run.color.gamma_multiply(faded),
                ..run
            }),
            12.0,
        );
    }
    if !text.is_empty() {
        c.text_dim(
            x + 32.0,
            y + 10.0 + title_height,
            text_width,
            &body_runs,
            19.0,
            faded,
            &mut used,
        );
    }
    if c.live && c.ring && response.has_focus() {
        c.focus_ring(rect, 2.0, None);
    }
    (height, enabled && c.live && response.clicked())
}

/// A chip of the welcome screen; returns its width.
pub(crate) fn chip(c: &mut Canvas<'_>, x: f32, y: f32, text: &str) -> f32 {
    let runs = [Run::mono(text, 12.0, 400.0, c.pal.text_soft)];
    let width = c.measure(&runs, f32::INFINITY, 16.0).x + 20.0 + 2.0;
    let rect = c.rect(x, y, width, 28.0);
    bordered_rect(
        &c.painter,
        rect,
        c.col(c.pal.line),
        c.col(c.pal.graphite),
        1.0,
    );
    c.text(x + 11.0, y + 6.0, width, &runs, 16.0);
    width
}

/// The field of a path: a border that is gold when the field has the focus and `alert` when the value is invalid.
/// Returns the response of the text edit.
pub(crate) fn path_field(
    c: &mut Canvas<'_>,
    id: &str,
    rect: Rect,
    value: &mut String,
    invalid: bool,
    label: &str,
) -> egui::Response {
    let pal = c.pal;
    let edit_id = c.ui.scope_id().with(id);
    let focused = c.ui.ctx().memory(|memory| memory.has_focus(edit_id));
    let border = if invalid {
        pal.alert
    } else if focused {
        pal.gold
    } else {
        pal.control_edge
    };
    bordered_rect(
        &c.painter,
        rect,
        c.col(border),
        c.col(pal.graphite_soft),
        1.0,
    );
    let inner = rect.shrink2(vec2(12.0, 0.0)).shrink2(vec2(0.0, 1.0));
    let mut response = None;
    if c.live {
        c.ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            let visuals = ui.visuals_mut();
            visuals.selection.bg_fill = pal.gold.gamma_multiply(0.35);
            visuals.selection.stroke = egui::Stroke::new(1.0, pal.text_strong);
            visuals.text_cursor.stroke = egui::Stroke::new(1.5, pal.gold);
            let edit = TextEdit::singleline(value)
                .id(edit_id)
                .font(theme::mono(13.0))
                .text_color(pal.text_strong)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .align(egui::Align2::LEFT_CENTER)
                .desired_width(inner.width());
            let output = ui.add_sized(inner.size(), edit);
            output.widget_info(|| WidgetInfo::labeled(Role::TextInput, true, label));
            response = Some(output);
        });
    }
    response.unwrap_or_else(|| c.interact(id, rect, Sense::hover()))
}

/// The progress of 28 chevrons, with the value in percent for a reader of the screen.
pub(crate) fn progress_chevrons(c: &mut Canvas<'_>, x: f32, y: f32, fraction: f32, label: &str) {
    const COUNT: usize = 28;
    let pal = c.pal;
    let filled = (fraction.clamp(0.0, 1.0) * COUNT as f32).floor() as usize;
    for index in 0..COUNT {
        let color = if index < filled {
            pal.gold
        } else if index == filled && fraction < 1.0 {
            pal.silver
        } else {
            pal.line
        };
        shape::chevron_progress(
            &c.painter,
            c.point(x + index as f32 * 17.0, y),
            c.col(color),
        );
    }
    let rect = c.rect(x, y, COUNT as f32 * 14.0 + (COUNT as f32 - 1.0) * 3.0, 22.0);
    if c.live {
        let response =
            c.ui.interact(rect, c.ui.scope_id().with("progress"), Sense::hover());
        let percent = (fraction * 100.0).round();
        response.widget_info(|| {
            let mut info = WidgetInfo::new(Role::ProgressIndicator);
            info.label = Some(label.to_string());
            info.value = Some(f64::from(percent));
            info
        });
    }
}

/// The log: Geist Mono on `black-deep`, at most 8 lines, the newest at the bottom. Returns nothing; the lines are
/// the entries of the run.
pub(crate) fn log_view(c: &mut Canvas<'_>, rect: Rect, lines: &[LogLine], fixed_height: bool) {
    let pal = c.pal;
    bordered_rect(
        &c.painter,
        rect,
        c.col(pal.line),
        c.col(pal.black_deep),
        1.0,
    );
    let rows = if fixed_height { 8 } else { 64 };
    let start = lines.len().saturating_sub(rows);
    let visible = &lines[start..];
    let x = rect.min.x - c.origin.x + 14.0;
    let width = rect.width() - 28.0;
    let top = rect.min.y - c.origin.y - c.dy + 12.0;
    let bottom = rect.max.y - c.origin.y - c.dy - 12.0;
    let mut y = if fixed_height {
        bottom - visible.len() as f32 * 19.0
    } else {
        top
    };
    for line in visible {
        let runs = log_runs(line, &pal);
        if fixed_height {
            let galley = c.galley(&runs, f32::INFINITY, 19.0);
            c.painter.with_clip_rect(rect.shrink(1.0)).galley(
                c.point(x, y),
                galley,
                Color32::WHITE,
            );
            y += 19.0;
        } else {
            y += c.text(x, y, width, &runs, 19.0);
        }
    }
    if c.live {
        let response =
            c.ui.interact(rect, c.ui.scope_id().with("log"), Sense::hover());
        response.widget_info(|| WidgetInfo::labeled(Role::Label, true, "Log"));
        c.ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_live(egui::accesskit::Live::Polite)
        });
    }
}

/// The runs of a line of the log, in the syntax of the terminal of `nurt`.
pub(crate) fn log_runs(line: &LogLine, pal: &super::theme::Palette) -> Vec<Run> {
    match line {
        LogLine::Command { name, arg } => {
            let mut runs = vec![
                Run::mono("nurt", 12.0, 400.0, pal.gold),
                Run::mono("> ", 12.0, 400.0, pal.text_soft),
                Run::mono(name.clone(), 12.0, 700.0, pal.gold),
            ];
            if !arg.is_empty() {
                runs.push(Run::mono(" ", 12.0, 400.0, pal.text_soft));
                runs.push(Run::mono(format!("\"{arg}\""), 12.0, 400.0, pal.cyan));
            }
            runs
        }
        LogLine::Answer { text, tone } => {
            let color = match tone {
                Tone::Plain => pal.text_soft,
                Tone::Ok => pal.silver,
                Tone::Alert => pal.alert,
            };
            vec![Run::mono(format!("  {text}"), 12.0, 400.0, color)]
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
