// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/screens.rs
// Responsibility:   Screens of the wizard: Welcome, Licence, Location, Options, Review, Install, Finish, Error, Maintenance and Removed.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Screens of the wizard (section 8 of `design/installer/installer.md`).
//!
//! Every screen is drawn at the coordinates of the design: the content starts at x = 296 (the rail
//! of 248 and the margin of 48) and is 616 points wide; the heading takes the eyebrow, the title and
//! the lead in the sizes of section 5. The texts are English, as the rest of the product.

use eframe::egui::{self, Align2, Key, Rect, Sense, pos2, vec2};

use super::draw::{Canvas, Run};
use super::shape::{self, bordered_cut};
use super::widgets::{self, ButtonKind};
use super::{App, Maintenance, RunMode, Screen};
use crate::model::{Issue, LogLine, Scope, StageState, Tone};
use crate::{app, engine, paths, requirements};

/// Left edge of the content.
const X: f32 = 296.0;
/// Width of the content.
const W: f32 = 616.0;
/// Top of the body area.
const TOP: f32 = 44.0;

/// Name of a stage for the line above the log.
pub(crate) fn stage_label(mode: RunMode, stage: &str, state: StageState) -> String {
    if state == StageState::Rollback {
        return "Undoing the changes".to_string();
    }
    match (mode, stage) {
        (RunMode::Install, "verify") => "Verifying SHA-256",
        (RunMode::Install, "extract") => "Extracting",
        (RunMode::Install, "copy") => "Copying files",
        (RunMode::Install, "path") => "Updating PATH",
        (RunMode::Install, "shortcuts") => "Creating shortcuts and profiles",
        (RunMode::Install, "journal") => "Writing the journal",
        (RunMode::Undo, "read") => "Reading the journal",
        (RunMode::Undo, "remove") => "Removing files",
        (RunMode::Undo, "restore") => "Restoring PATH",
        (RunMode::Undo, "close") => "Closing the journal",
        _ => "Starting",
    }
    .to_string()
}

/// A line of the log as plain text.
pub(crate) fn log_text(line: &LogLine) -> String {
    match line {
        LogLine::Command { name, arg } => {
            if arg.is_empty() {
                format!("nurt> {name}")
            } else {
                format!("nurt> {name} \"{arg}\"")
            }
        }
        LogLine::Answer { text, .. } => format!("  {text}"),
    }
}

/// Size in megabytes as the design writes it: `12 MB`.
fn megabytes(bytes: u64) -> String {
    format!("{} MB", ((bytes as f64) / 1_000_000.0).ceil().max(1.0))
}

fn flow_label(app: &App) -> String {
    format!("Step {} of {}", app.index + 1, app.steps().len())
}

impl App {
    /// Draws one screen; `current` is false for a screen that is leaving.
    pub(crate) fn draw_screen(&mut self, c: &mut Canvas<'_>, screen: Screen, current: bool) {
        let bottom = match screen {
            Screen::Welcome => self.screen_welcome(c),
            Screen::Licence => self.screen_licence(c),
            Screen::Location => self.screen_location(c),
            Screen::Options => self.screen_options(c),
            Screen::Review => self.screen_review(c),
            Screen::Install => self.screen_install(c),
            Screen::Finish => self.screen_finish(c),
            Screen::Error => self.screen_error(c),
            Screen::Maintenance => self.screen_maintenance(c),
            Screen::Removed => self.screen_removed(c),
        };
        if current {
            c.dy += self.scroll;
            self.draw_scrollbar(c, bottom);
            c.dy -= self.scroll;
        }
    }

    /// The eyebrow, the title and the lead. Returns the y where the content begins.
    fn heading(
        &self,
        c: &mut Canvas<'_>,
        eyebrow: &str,
        alert: bool,
        title: &str,
        lead: &[Run],
    ) -> f32 {
        let pal = c.pal;
        let color = if alert { pal.alert } else { pal.gold };
        c.text(
            X,
            TOP + 6.0,
            W,
            &[Run::sans(eyebrow.to_uppercase(), 12.0, 500.0, color).tracking(100.0)],
            16.0,
        );
        let title_y = TOP + 6.0 + 16.0 + 10.0;
        c.text(
            X,
            title_y,
            W,
            &[Run::sans(title, 30.0, 600.0, pal.text_strong).tracking(-12.0)],
            36.0,
        );
        let lead_y = title_y + 36.0 + 8.0;
        let lead_height = if lead.is_empty() {
            0.0
        } else {
            c.text(X, lead_y, 540.0, lead, 22.0)
        };
        lead_y + lead_height + 24.0
    }

    /// The heading with the mark at the left of the title (Finish and Removed). Returns the y where the content begins.
    fn seal_heading(&self, c: &mut Canvas<'_>, eyebrow: &str, title: &str, lead: &[Run]) -> f32 {
        let pal = c.pal;
        c.text(
            X,
            TOP + 6.0,
            W,
            &[Run::sans(eyebrow.to_uppercase(), 12.0, 500.0, pal.gold).tracking(100.0)],
            16.0,
        );
        let block_y = TOP + 6.0 + 16.0 + 10.0;
        if let Some(art) = &self.art {
            let rect = Rect::from_min_size(c.point(X, block_y + 3.0), vec2(56.0, 56.0));
            c.painter.image(
                art.seal.texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                c.col(eframe::egui::Color32::WHITE),
            );
        }
        c.text(
            X + 56.0 + 16.0,
            block_y,
            W - 72.0,
            &[Run::sans(title, 30.0, 600.0, pal.text_strong).tracking(-12.0)],
            36.0,
        );
        c.text(X + 56.0 + 16.0, block_y + 36.0 + 4.0, W - 72.0, lead, 22.0);
        block_y + 62.0 + 20.0
    }

    // ------------------------------------------------------------------------- welcome

    fn screen_welcome(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let lead = [Run::sans(
            "Command shell and language where every change can be undone.",
            15.0,
            400.0,
            pal.text_soft,
        )];
        let mut y = self.heading(c, "Setup", false, "Install Danaco Nurt", &lead);
        let machine = self.options.scope == Scope::Machine;
        let items: [(Vec<Run>, f32); 3] = [
            (
                vec![
                    Run::sans(
                        format!("nurt.exe {}", app::VERSION),
                        15.0,
                        600.0,
                        pal.text_strong,
                    ),
                    Run::sans(
                        match (self.options.docs, machine) {
                            (true, true) => " and its documentation are copied to this PC.",
                            (true, false) => " and its documentation are copied to your account.",
                            (false, true) => " is copied to this PC.",
                            (false, false) => " is copied to your account.",
                        },
                        15.0,
                        400.0,
                        pal.text_body,
                    ),
                ],
                0.0,
            ),
            (
                vec![
                    Run::sans("nurt", 15.0, 600.0, pal.text_strong),
                    Run::sans(
                        " is added to your PATH, so any terminal can start it.",
                        15.0,
                        400.0,
                        pal.text_body,
                    ),
                ],
                0.0,
            ),
            (
                vec![
                    Run::sans("Every change is journaled.", 15.0, 600.0, pal.text_strong),
                    Run::sans(
                        " You can undo this setup at any time.",
                        15.0,
                        400.0,
                        pal.text_body,
                    ),
                ],
                0.0,
            ),
        ];
        let heights: Vec<f32> = items
            .iter()
            .map(|(runs, _)| {
                c.measure(runs, W - 2.0 - 40.0 - 18.0 - 14.0, 22.0)
                    .y
                    .max(22.0)
            })
            .collect();
        let list_height: f32 = heights.iter().map(|h| h + 16.0).sum::<f32>() + 2.0;
        let card_height = 2.0 + 36.0 + list_height;
        widgets::card(c, c.rect(X, y, W, card_height));
        let mut row = y + 1.0 + 18.0;
        for (index, (runs, _)) in items.iter().enumerate() {
            if index > 0 {
                c.painter
                    .rect_filled(c.rect(X + 21.0, row, W - 42.0, 1.0), 0.0, c.col(pal.line));
                row += 1.0;
            }
            shape::chevron_marker(&c.painter, c.point(X + 21.0, row + 8.0), c.col(pal.gold));
            c.text(
                X + 21.0 + 18.0 + 14.0,
                row + 8.0,
                W - 2.0 - 40.0 - 18.0 - 14.0,
                runs,
                22.0,
            );
            row += heights[index] + 16.0;
        }
        y += card_height + 18.0;
        let mut x = X;
        let chips = [
            format!("Windows {}", requirements::setup_arch()),
            if machine {
                "for all users of this PC".to_string()
            } else {
                "for the current user".to_string()
            },
            if machine {
                "administrator approval needed".to_string()
            } else {
                "no administrator rights needed".to_string()
            },
        ];
        for chip in &chips {
            x += widgets::chip(c, x, y, chip) + 8.0;
        }
        y += 28.0;
        if requirements::blocked(&self.requirements) {
            y += 18.0;
            let mut lines = vec![LogLine::command("check", "requirements")];
            for row in self.requirements.iter().filter(|row| !row.ok) {
                lines.push(LogLine::answer(
                    requirement_text(row.id, &row.found, &row.required),
                    Tone::Alert,
                ));
            }
            let height = 24.0 + 19.0 * lines.len() as f32 + 2.0;
            widgets::log_view(c, c.rect(X, y, W, height), &lines, false);
            y += height;
            c.text(
                X,
                y + 14.0,
                W,
                &[Run::sans(
                    "Setup cannot continue on this PC. Nothing was changed.",
                    13.0,
                    400.0,
                    pal.alert,
                )],
                19.0,
            );
            y += 14.0 + 19.0;
        }
        y
    }

    // ------------------------------------------------------------------------- licence

    fn screen_licence(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let lead = [Run::sans(
            "Danaco Nurt is released under the Apache License, Version 2.0. Read the licence to the end to continue.",
            15.0,
            400.0,
            pal.text_soft,
        )];
        let eyebrow = flow_label(self);
        let y = self.heading(c, &eyebrow, false, "Licence", &lead);
        let rect = c.rect(X, y, W, 236.0);
        shape::bordered_rect(
            &c.painter,
            rect,
            c.col(pal.line),
            c.col(pal.graphite_soft),
            1.0,
        );
        let inner = rect.shrink2(vec2(16.0, 14.0));
        let runs = [Run::mono(
            self.licence.text.clone(),
            12.0,
            400.0,
            pal.text_soft,
        )];
        let galley = c.galley(&runs, inner.width() - 12.0, 19.0);
        self.licence.content = galley.size().y;
        let view = 236.0 - 28.0;
        let max = (self.licence.content - view).max(0.0);
        let response = c.interact("licence-text", rect, Sense::click_and_drag());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ScrollView, true, "Licence text")
        });
        if c.live {
            let hovered =
                c.ui.input(|input| input.pointer.hover_pos())
                    .is_some_and(|pos| rect.contains(pos));
            let delta = if hovered {
                c.ui.input(|input| input.smooth_scroll_delta.y)
            } else {
                0.0
            };
            let keys = if response.has_focus() || hovered {
                c.ui.input(|input| {
                    let mut step = 0.0;
                    if input.key_pressed(Key::ArrowDown) {
                        step += 19.0;
                    }
                    if input.key_pressed(Key::ArrowUp) {
                        step -= 19.0;
                    }
                    if input.key_pressed(Key::PageDown) || input.key_pressed(Key::Space) {
                        step += view - 19.0;
                    }
                    if input.key_pressed(Key::PageUp) {
                        step -= view - 19.0;
                    }
                    if input.key_pressed(Key::End) {
                        step += max;
                    }
                    if input.key_pressed(Key::Home) {
                        step -= max;
                    }
                    step
                })
            } else {
                0.0
            };
            self.licence.scroll = (self.licence.scroll - delta + keys).clamp(0.0, max);
            if response.dragged() {
                self.licence.scroll =
                    (self.licence.scroll - response.drag_delta().y).clamp(0.0, max);
            }
            if max > 0.0 {
                let track = Rect::from_min_max(pos2(rect.max.x - 18.0, rect.min.y), rect.max);
                let track_response = c.interact("licence-track", track, Sense::click_and_drag());
                track_response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::ScrollBar, true, "Licence scroll bar")
                });
                if let Some(pointer) = track_response.interact_pointer_pos()
                    && (track_response.clicked() || track_response.dragged())
                {
                    let thumb_h = (view * view / self.licence.content).max(24.0);
                    let free = (view - thumb_h).max(1.0);
                    if track_response.dragged() {
                        let fraction =
                            ((pointer.y - inner.min.y - thumb_h / 2.0) / free).clamp(0.0, 1.0);
                        self.licence.scroll = fraction * max;
                    } else {
                        let thumb_top = inner.min.y + free * self.licence.scroll / max;
                        let page = view - 19.0;
                        let target = if pointer.y > thumb_top + thumb_h {
                            self.licence.scroll + page
                        } else if pointer.y < thumb_top {
                            self.licence.scroll - page
                        } else {
                            self.licence.scroll
                        };
                        self.licence.scroll = target.clamp(0.0, max);
                    }
                }
            }
        }
        if max <= 0.0 || self.licence.scroll >= max - 2.0 {
            self.licence.read = true;
        }
        c.painter
            .with_clip_rect(inner.expand2(vec2(0.0, 0.0)))
            .galley(
                pos2(inner.min.x, inner.min.y - self.licence.scroll),
                galley,
                eframe::egui::Color32::WHITE,
            );
        if max > 0.0 {
            let thumb_h = (view * view / self.licence.content).max(24.0);
            let thumb_y = inner.min.y + (view - thumb_h) * self.licence.scroll / max;
            c.painter.rect_filled(
                Rect::from_min_size(pos2(rect.max.x - 11.0, thumb_y), vec2(10.0, thumb_h)),
                0.0,
                c.col(pal.control_edge),
            );
        }
        if c.live && c.ring && response.has_focus() {
            c.focus_ring(rect, 3.0, None);
        }
        let note_y = y + 236.0 + 16.0;
        if self.licence.read {
            shape::tick(
                &c.painter,
                c.point(X + 2.0, note_y + 4.0),
                c.col(pal.silver),
            );
            c.text(
                X + 22.0,
                note_y,
                W - 22.0,
                &[Run::sans(
                    "You have read the licence to the end.",
                    13.0,
                    400.0,
                    pal.text_soft,
                )],
                19.0,
            );
        } else {
            let percent = if self.licence.content > 0.0 {
                ((self.licence.scroll + view) / self.licence.content * 100.0).min(99.0)
            } else {
                0.0
            };
            c.text(
                X,
                note_y,
                W,
                &[Run::sans(
                    format!("Scroll to the end of the licence to continue ({percent:.0} %)."),
                    13.0,
                    400.0,
                    pal.text_muted,
                )],
                19.0,
            );
        }
        note_y + 19.0
    }

    // ------------------------------------------------------------------------- location

    fn screen_location(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let lead = [Run::sans(
            "Choose who can use nurt and where its files go.",
            15.0,
            400.0,
            pal.text_soft,
        )];
        let eyebrow = flow_label(self);
        let mut y = self.heading(c, &eyebrow, false, "Where to install", &lead);
        let scopes = [
            (
                Scope::User,
                "Only for me",
                "No administrator rights needed. Recommended.",
                "user",
            ),
            (
                Scope::Machine,
                "For all users of this PC",
                "Changes the PATH of the machine. Windows asks for administrator approval.",
                "machine",
            ),
        ];
        let mut chosen = None;
        for (index, (scope, title, text, tag)) in scopes.iter().enumerate() {
            let selected = self.options.scope == *scope;
            let outcome = widgets::choice(
                c,
                &format!("scope-{tag}"),
                c.rect(X, y, W, 73.0),
                title,
                text,
                tag,
                selected,
                true,
            );
            if outcome.clicked {
                chosen = Some(*scope);
            }
            if outcome.focused && c.live {
                let (down, up) = c.ui.input(|input| {
                    (
                        input.key_pressed(Key::ArrowDown),
                        input.key_pressed(Key::ArrowUp),
                    )
                });
                if down || up {
                    let next =
                        (index + usize::from(down) + scopes.len() - usize::from(up)) % scopes.len();
                    chosen = Some(scopes[next].0);
                    let id = c.ui.scope_id().with(format!("scope-{}", scopes[next].3));
                    c.ui.ctx().memory_mut(|memory| memory.request_focus(id));
                }
            }
            y += 73.0 + 10.0;
        }
        if let Some(scope) = chosen
            && scope != self.options.scope
        {
            self.options.scope = scope;
            self.options.dir = String::new();
            self.refresh_location(true);
        }
        y += 18.0;
        c.text(
            X,
            y,
            W,
            &[Run::sans("Folder of nurt.exe", 13.0, 400.0, pal.text_soft)],
            22.0,
        );
        y += 22.0 + 6.0;
        let browse_width = widgets::button_width(c, "Browse", ButtonKind::Secondary, false);
        let input_rect = c.rect(X, y, W - 10.0 - browse_width, 40.0);
        let mut text = if self.options.dir.is_empty() {
            self.location.dir.clone()
        } else {
            self.options.dir.clone()
        };
        let shown = text.clone();
        let invalid = self.location.issue.is_some();
        let response = widgets::path_field(
            c,
            "folder",
            input_rect,
            &mut text,
            invalid,
            "Folder of nurt.exe",
        );
        if c.live && response.changed() && text != shown {
            self.options.dir = text;
            self.refresh_location(true);
        }
        if widgets::button(
            c,
            "browse",
            c.rect(X + W - browse_width, y, browse_width, 40.0),
            "Browse",
            ButtonKind::Secondary,
            false,
            true,
        ) {
            let start = self.location.dir.clone();
            if let Some(picked) = crate::sys::pick_folder("Choose the folder of nurt.exe", &start) {
                self.options.dir = folder_from_picked(&picked);
                self.refresh_location(true);
            }
        }
        y += 40.0 + 6.0;
        let hint = self.location_hint();
        let color = if invalid { pal.alert } else { pal.text_muted };
        c.text(X, y, W, &[Run::sans(hint, 13.0, 400.0, color)], 18.0);
        y + 18.0
    }

    fn location_hint(&self) -> String {
        match self.location.issue {
            Some(Issue::NotFullPath) => "Enter a full path, for example C:\\Tools\\Nurt\\bin.".to_string(),
            Some(Issue::OutsideProgramFiles) => "For all users the folder must be inside C:\\Program Files: every account runs nurt.exe from it.".to_string(),
            Some(Issue::IsFile) => "This path is a file. Enter the path of a folder.".to_string(),
            Some(Issue::NotWritable) => "Setup cannot write to this folder. Choose another folder.".to_string(),
            Some(Issue::NoSpace) => format!("There is not enough free space on this disk: {} needed.", megabytes(self.location.required)),
            None => format!(
                "About {} needed. {}",
                megabytes(self.location.required),
                if self.options.scope == Scope::Machine { "Windows asks for administrator approval when the install starts." } else { "No administrator rights needed." }
            ),
        }
    }

    // ------------------------------------------------------------------------- options

    fn screen_options(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let lead = [Run::sans(
            "Everything here can be changed later by running the setup again.",
            15.0,
            400.0,
            pal.text_soft,
        )];
        let eyebrow = flow_label(self);
        let y = self.heading(c, &eyebrow, false, "Options", &lead);
        let wt = paths::terminal_installed(&self.startup.user);
        let docs_possible = crate::model::Layout::of(
            &crate::location::resolve(&self.options, &self.startup.user),
            true,
        )
        .docs;
        struct Row {
            id: &'static str,
            title: &'static str,
            new: bool,
            text: String,
            enabled: bool,
        }
        let rows = [
            Row { id: "path", title: "Add nurt to PATH", new: false, text: "Any new terminal can start nurt.".into(), enabled: true },
            Row {
                id: "docs",
                title: "Install the offline documentation",
                new: false,
                text: if docs_possible { "About 200 HTML pages in the doc folder next to bin. The program does not need them.".into() } else { "Available when the folder of nurt.exe is named bin.".into() },
                enabled: docs_possible,
            },
            Row {
                id: "shortcut",
                title: "Start Menu shortcut",
                new: true,
                text: "Opens a terminal with nurt. To pin it to the taskbar, use Pin to taskbar in the Start menu: Windows does not let a setup do it.".into(),
                enabled: true,
            },
            Row { id: "terminal", title: "Windows Terminal profile", new: true, text: "Adds a profile named Danaco Nurt, in the colours of nurt.".into(), enabled: true },
            Row {
                id: "default",
                title: "Make it the default profile",
                new: false,
                text: if wt { "Windows Terminal opens the Danaco Nurt profile first.".into() } else { "Windows Terminal is not installed for your account.".into() },
                enabled: self.options.terminal_profile && wt,
            },
            Row { id: "desktop", title: "Desktop shortcut", new: false, text: "A shortcut named Danaco Nurt on the desktop.".into(), enabled: true },
            Row { id: "quick", title: "Quick Launch shortcut", new: false, text: "A shortcut in the Quick Launch folder of your account.".into(), enabled: true },
            Row { id: "completion", title: "Tab completion in PowerShell", new: false, text: "Completes the commands of nurt with the Tab key.".into(), enabled: true },
        ];
        let checked = |id: &str, options: &crate::model::Options| match id {
            "path" => options.path,
            "docs" => options.docs && docs_possible,
            "shortcut" => options.start_menu,
            "terminal" => options.terminal_profile,
            "default" => options.terminal_default && options.terminal_profile && wt,
            "desktop" => options.desktop,
            "quick" => options.quick_launch,
            _ => options.completions,
        };
        // Measure the card first, so that the border is drawn below the checks.
        let mut heights = Vec::new();
        for row in &rows {
            let title = [Run::sans(row.title, 15.0, 400.0, pal.text_strong)];
            let body = [Run::sans(row.text.clone(), 13.0, 400.0, pal.text_muted)];
            heights.push(
                20.0 + c.measure(&title, W - 40.0 - 32.0, 22.0).y
                    + c.measure(&body, W - 40.0 - 32.0, 19.0).y,
            );
        }
        let verify_title = [Run::sans(
            "Verify the package against checksums.txt",
            15.0,
            400.0,
            pal.text_strong,
        )];
        let verify_text =
            "Always on. The SHA-256 digest of the package is checked before anything is installed.";
        let verify_height = 20.0
            + c.measure(&verify_title, W - 72.0, 22.0).y
            + c.measure(
                &[Run::sans(verify_text, 13.0, 400.0, pal.text_muted)],
                W - 72.0,
                19.0,
            )
            .y;
        let total: f32 =
            heights.iter().sum::<f32>() + verify_height + rows.len() as f32 + 16.0 + 2.0;
        widgets::card(c, c.rect(X, y, W, total));
        let mut row_y = y + 1.0 + 8.0;
        for (index, row) in rows.iter().enumerate() {
            if index > 0 {
                c.painter
                    .rect_filled(c.rect(X + 21.0, row_y, W - 42.0, 1.0), 0.0, c.col(pal.line));
                row_y += 1.0;
            }
            let is_checked = checked(row.id, &self.options);
            let (height, pressed) = widgets::check(
                c,
                &format!("option-{}", row.id),
                X + 21.0,
                row_y,
                W - 42.0,
                row.title,
                row.new,
                &row.text,
                is_checked,
                row.enabled,
            );
            if pressed {
                match row.id {
                    "path" => self.options.path = !self.options.path,
                    "docs" => self.options.docs = !self.options.docs,
                    "shortcut" => self.options.start_menu = !self.options.start_menu,
                    "terminal" => {
                        self.options.terminal_profile = !self.options.terminal_profile;
                        if !self.options.terminal_profile {
                            self.options.terminal_default = false;
                        }
                    }
                    "default" => self.options.terminal_default = !self.options.terminal_default,
                    "desktop" => self.options.desktop = !self.options.desktop,
                    "quick" => self.options.quick_launch = !self.options.quick_launch,
                    _ => self.options.completions = !self.options.completions,
                }
            }
            row_y += height;
        }
        c.painter
            .rect_filled(c.rect(X + 21.0, row_y, W - 42.0, 1.0), 0.0, c.col(pal.line));
        row_y += 1.0;
        let (height, _) = widgets::check(
            c,
            "option-verify",
            X + 21.0,
            row_y,
            W - 42.0,
            "Verify the package against checksums.txt",
            false,
            verify_text,
            true,
            false,
        );
        row_y += height;
        let bottom = row_y + 8.0 + 1.0;

        // The first configuration of the program.
        let config_top = bottom + 14.0;
        let config_rows = 3.0 * 62.0;
        let config_height = 2.0
            + 16.0
            + 62.0
            + if self.options.write_config {
                12.0 + config_rows
            } else {
                0.0
            };
        widgets::card(c, c.rect(X, config_top, W, config_height));
        let (_, toggled) = widgets::check(
            c,
            "option-config",
            X + 21.0,
            config_top + 1.0,
            W - 42.0,
            "Write the first configuration",
            false,
            "Creates nurt.toml in %APPDATA%\\Nurt. An existing file is never changed.",
            self.options.write_config,
            true,
        );
        if toggled {
            self.options.write_config = !self.options.write_config;
        }
        if self.options.write_config {
            let mut line_y = config_top + 1.0 + 62.0 + 8.0;
            // Every choice: the value written to nurt.toml, the text of its button and the line
            // that explains it below the buttons while it is selected.
            let groups: [(&str, &str, [(&str, &str, &str); 3]); 3] = [
                (
                    "Language of the messages",
                    "language",
                    [
                        (
                            "en",
                            "English",
                            "Messages, errors and help of nurt in English.",
                        ),
                        (
                            "pl",
                            "Polski",
                            "Messages, errors and help of nurt in Polish.",
                        ),
                        ("", "", ""),
                    ],
                ),
                (
                    "Colours of the output",
                    "color",
                    [
                        (
                            "auto",
                            "auto",
                            "Colours in a terminal only; files and pipes get plain text.",
                        ),
                        (
                            "always",
                            "always",
                            "Colours everywhere, also in files and pipes.",
                        ),
                        ("never", "never", "Plain text everywhere."),
                    ],
                ),
                (
                    "How programs are written",
                    "syntax",
                    [
                        (
                            "a",
                            "words",
                            "find \"C:\\logs\" glob: \"*.log\"  - arguments follow the command (default)",
                        ),
                        (
                            "b",
                            "calls",
                            "find(\"C:\\logs\", glob: \"*.log\")  - arguments in parentheses",
                        ),
                        (
                            "c",
                            "names",
                            "find path=\"C:\\logs\" glob=\"*.log\"  - every argument by name",
                        ),
                    ],
                ),
            ];
            for (label, key, choices) in groups {
                c.text(
                    X + 21.0,
                    line_y + 9.0,
                    220.0,
                    &[Run::sans(label, 13.0, 400.0, pal.text_soft)],
                    22.0,
                );
                let mut x = X + 21.0 + 240.0;
                for (value, text, _) in choices.iter().filter(|(value, _, _)| !value.is_empty()) {
                    let current = match key {
                        "language" => &self.options.config.language,
                        "color" => &self.options.config.color,
                        _ => &self.options.config.syntax,
                    };
                    let selected = current == value;
                    let width = 76.0;
                    if segment(
                        c,
                        &format!("config-{key}-{value}"),
                        c.rect(x, line_y + 2.0, width, 32.0),
                        text,
                        selected,
                        label,
                    ) {
                        match key {
                            "language" => self.options.config.language = (*value).to_string(),
                            "color" => self.options.config.color = (*value).to_string(),
                            _ => self.options.config.syntax = (*value).to_string(),
                        }
                    }
                    x += width + 8.0;
                }
                let current = match key {
                    "language" => &self.options.config.language,
                    "color" => &self.options.config.color,
                    _ => &self.options.config.syntax,
                };
                if let Some((_, _, hint)) = choices.iter().find(|(value, _, _)| current == value) {
                    c.text(
                        X + 21.0,
                        line_y + 40.0,
                        W - 42.0,
                        &[Run::sans(*hint, 12.0, 400.0, pal.text_soft)],
                        18.0,
                    );
                }
                line_y += 62.0;
            }
        }
        config_top + config_height
    }

    // -------------------------------------------------------------------------- review

    fn screen_review(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let lead = [Run::sans(
            "These changes will be made and written to the journal.",
            15.0,
            400.0,
            pal.text_soft,
        )];
        let eyebrow = flow_label(self);
        let mut y = self.heading(c, &eyebrow, false, "Ready to install", &lead);
        let bin = crate::location::resolve(&self.options, &self.startup.user);
        let rows = [
            (
                "For",
                if self.options.scope == Scope::Machine {
                    "all users of this PC (administrator)".to_string()
                } else {
                    "the current user".to_string()
                },
            ),
            ("Folder", bin.display().to_string()),
            (
                "Package",
                format!(
                    "nurt-{}-{}.zip, checked against checksums.txt",
                    app::VERSION,
                    app::TARGET
                ),
            ),
        ];
        let heights: Vec<f32> = rows
            .iter()
            .map(|(_, value)| {
                c.measure(
                    &[Run::mono(value.clone(), 13.0, 400.0, pal.text_strong)],
                    W - 42.0 - 150.0,
                    20.0,
                )
                .y
                .max(20.0)
                    + 14.0
            })
            .collect();
        let card_height = 2.0 + 20.0 + heights.iter().sum::<f32>() + 2.0;
        widgets::card(c, c.rect(X, y, W, card_height));
        let mut row_y = y + 1.0 + 10.0;
        for (index, (label, value)) in rows.iter().enumerate() {
            if index > 0 {
                c.painter
                    .rect_filled(c.rect(X + 21.0, row_y, W - 42.0, 1.0), 0.0, c.col(pal.line));
                row_y += 1.0;
            }
            c.text(
                X + 21.0,
                row_y + 7.0,
                140.0,
                &[Run::sans(*label, 14.0, 400.0, pal.text_muted)],
                20.0,
            );
            c.text(
                X + 21.0 + 150.0,
                row_y + 7.0,
                W - 42.0 - 150.0,
                &[Run::mono(value.clone(), 13.0, 400.0, pal.text_strong)],
                20.0,
            );
            row_y += heights[index];
        }
        y += card_height + 14.0;
        c.text(
            X,
            y,
            W,
            &[Run::sans("Journal", 13.0, 400.0, pal.text_muted)],
            19.0,
        );
        y += 19.0 + 8.0;
        for (index, entry) in engine::plan(&self.options, &self.startup.user)
            .iter()
            .enumerate()
        {
            c.text_at(
                Align2::RIGHT_TOP,
                X + 18.0,
                y,
                &[Run::mono((index + 1).to_string(), 12.0, 400.0, pal.gold)],
                20.0,
            );
            y += c.text(
                X + 30.0,
                y,
                W - 30.0,
                &[Run::mono(entry.clone(), 12.0, 400.0, pal.text_soft)],
                20.0,
            );
        }
        y
    }

    // -------------------------------------------------------------------------- install

    fn screen_install(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let undo = self.run.mode == RunMode::Undo;
        let (eyebrow, title) = if undo {
            ("Undo this setup".to_string(), "Removing")
        } else {
            (flow_label(self), "Installing")
        };
        let lead_text = if self.run.rollback {
            "The changes made so far are being undone."
        } else if undo {
            "Please wait. The journal of this setup is being reversed."
        } else {
            "Please wait. You can cancel; the changes made so far will be undone."
        };
        let lead = [Run::sans(lead_text, 15.0, 400.0, pal.text_soft)];
        let y = self.heading(c, &eyebrow, false, title, &lead);
        let chevrons_y = y + 8.0;
        widgets::progress_chevrons(
            c,
            X,
            chevrons_y,
            self.run.shown,
            if undo {
                "Progress of the undo"
            } else {
                "Progress of the installation"
            },
        );
        let line_y = chevrons_y + 22.0 + 8.0;
        let stage = if self.run.stage.is_empty() {
            "Starting".to_string()
        } else {
            self.run.stage.clone()
        };
        let stage = if self.run.end.as_ref().is_some_and(|end| end.ok) {
            "Done".to_string()
        } else {
            stage
        };
        c.text(
            X,
            line_y,
            W,
            &[Run::sans(stage, 13.0, 400.0, pal.text_soft)],
            22.0,
        );
        c.text_at(
            Align2::RIGHT_TOP,
            X + W,
            line_y,
            &[Run::mono(
                format!("{} %", (self.run.shown * 100.0).round()),
                13.0,
                600.0,
                pal.text_strong,
            )],
            22.0,
        );
        let log_y = line_y + 22.0 + 14.0;
        let rect = c.rect(X, log_y, W, 168.0);
        widgets::log_view(c, rect, &self.run.log, true);
        log_y + 168.0
    }

    // --------------------------------------------------------------------------- finish

    fn screen_finish(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let eyebrow = flow_label(self);
        let lead = [Run::sans(
            format!(
                "nurt {} is ready. Open a new terminal to use it.",
                app::VERSION
            ),
            15.0,
            400.0,
            pal.text_soft,
        )];
        let mut y = self.seal_heading(c, &eyebrow, "Danaco Nurt is installed", &lead);
        let summary = self.summary.clone().unwrap_or_default();
        let mut lines = vec![
            LogLine::command("install", &format!("{}\\nurt.exe", summary.bin)),
            LogLine::answer(
                format!(
                    "installed nurt {}  -  {} journal entries",
                    summary.version, summary.entries
                ),
                Tone::Ok,
            ),
        ];
        if summary.path_changed {
            lines.push(LogLine::answer(
                format!(
                    "added {} to the {} PATH; open a new terminal to use nurt",
                    summary.bin, summary.scope
                ),
                Tone::Plain,
            ));
        }
        for warning in &summary.warnings {
            lines.push(LogLine::answer(format!("warning  {warning}"), Tone::Plain));
        }
        let heights: f32 = lines
            .iter()
            .map(|line| c.measure(&widgets::log_runs(line, &pal), W - 30.0, 19.0).y)
            .sum();
        let height = 24.0 + 2.0 + heights;
        widgets::log_view(c, c.rect(X, y, W, height), &lines, false);
        y += height + 14.0;
        let card_height = 2.0 + 12.0 + 2.0 * 42.0 + 1.0;
        widgets::card(c, c.rect(X, y, W, card_height));
        let docs = std::path::Path::new(&summary.root)
            .join("doc")
            .join("index.html")
            .is_file();
        let (_, first) = widgets::check(
            c,
            "finish-start",
            X + 21.0,
            y + 1.0 + 6.0,
            W - 42.0,
            "Open a terminal and start nurt",
            false,
            "",
            self.finish_checks[0],
            true,
        );
        if first {
            self.finish_checks[0] = !self.finish_checks[0];
        }
        c.painter.rect_filled(
            c.rect(X + 21.0, y + 1.0 + 6.0 + 42.0, W - 42.0, 1.0),
            0.0,
            c.col(pal.line),
        );
        let (_, second) = widgets::check(
            c,
            "finish-docs",
            X + 21.0,
            y + 1.0 + 6.0 + 43.0,
            W - 42.0,
            if docs {
                "Show the documentation"
            } else {
                "Documentation not installed"
            },
            false,
            "",
            self.finish_checks[1] && docs,
            docs,
        );
        if second {
            self.finish_checks[1] = !self.finish_checks[1];
        }
        y + card_height
    }

    // ---------------------------------------------------------------------------- error

    fn screen_error(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let failure = self.failure.clone();
        let Some(failure) = failure else { return TOP };
        let (title, safe) = error_texts(&failure.code);
        let nothing = failure.undone == 0;
        // A failed read of the journal (stage `read`) stops the undo before it changes anything.
        let incomplete = !failure.rolled_back && failure.stage != "read";
        let lead = [
            Run::sans(safe, 15.0, 400.0, pal.text_soft),
            Run::sans(
                if incomplete {
                    " Some changes could not be undone."
                } else if nothing {
                    " Nothing on this PC was changed."
                } else {
                    " The changes made so far were undone."
                },
                15.0,
                600.0,
                pal.text_strong,
            ),
        ];
        let mut y = self.heading(c, "Setup stopped", true, title, &lead);
        let mut lines = vec![LogLine::command(&failure.stage, "")];
        lines.push(LogLine::answer(
            if failure.detail.is_empty() {
                failure.code.clone()
            } else {
                format!("{}  {}", failure.code, failure.detail)
            },
            Tone::Alert,
        ));
        lines.push(LogLine::answer(
            if incomplete {
                "the rollback was incomplete; see the log file of the setup".to_string()
            } else if !failure.rolled_back {
                "the journal could not be read; nothing was removed".to_string()
            } else if nothing {
                "the setup journal is empty".to_string()
            } else {
                format!("entries 1 to {} were undone", failure.undone)
            },
            Tone::Plain,
        ));
        let heights: f32 = lines
            .iter()
            .map(|line| c.measure(&widgets::log_runs(line, &pal), W - 30.0, 19.0).y)
            .sum();
        let height = 24.0 + 2.0 + heights;
        widgets::log_view(c, c.rect(X, y, W, height), &lines, false);
        y += height;
        y += 14.0;
        let note = "Try again. If it fails again, report it at support@danaco-group.pl and include the log.";
        y += c.text(
            X,
            y,
            W,
            &[Run::sans(note, 13.0, 400.0, pal.text_muted)],
            19.0,
        );
        if self.copied.is_some() {
            y += 8.0;
            y += c.text(
                X,
                y,
                W,
                &[Run::sans(
                    "The log was copied to the clipboard.",
                    13.0,
                    400.0,
                    pal.text_soft,
                )],
                19.0,
            );
        }
        y
    }

    // ----------------------------------------------------------------------- maintenance

    fn screen_maintenance(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let Some(existing) = self.existing.clone() else {
            return TOP;
        };
        let for_whom = if existing.scope == Scope::Machine {
            "all users of this PC"
        } else {
            "the current user"
        };
        let differs = existing.version != app::VERSION;
        let mut lead_text =
            format!("This setup found an installation made for {for_whom}. Choose what to do.");
        if differs {
            lead_text.push_str(&format!(" This setup installs {}.", app::VERSION));
        }
        let lead = [Run::sans(lead_text, 15.0, 400.0, pal.text_soft)];
        let title = format!("Danaco Nurt {} is installed", existing.version);
        let mut y = self.heading(c, "Maintenance", false, &title, &lead);
        let repair_title = if differs { "Update" } else { "Repair" };
        let repair_text = if differs {
            format!(
                "Install {} over {} and check every file against checksums.txt.",
                app::VERSION,
                existing.version
            )
        } else {
            "Install the same version again and check every file against checksums.txt.".to_string()
        };
        let undo_text = format!(
            "Reverse the journal entries 1 to {} and remove nurt from {}.",
            existing.entries.saturating_sub(1),
            if existing.scope == Scope::Machine {
                "this PC"
            } else {
                "this account"
            }
        );
        let entries = [
            (Maintenance::Repair, repair_title.to_string(), repair_text),
            (Maintenance::Undo, "Undo this setup".to_string(), undo_text),
        ];
        for (index, (kind, title, text)) in entries.iter().enumerate() {
            let selected = self.maintenance == *kind;
            let outcome = widgets::choice(
                c,
                &format!("maintenance-{index}"),
                c.rect(X, y, W, 73.0),
                title,
                text,
                "",
                selected,
                true,
            );
            if outcome.clicked {
                self.maintenance = *kind;
            }
            if outcome.focused && c.live {
                let (down, up) = c.ui.input(|input| {
                    (
                        input.key_pressed(Key::ArrowDown),
                        input.key_pressed(Key::ArrowUp),
                    )
                });
                if down || up {
                    let next = (index + usize::from(down) + entries.len() - usize::from(up))
                        % entries.len();
                    self.maintenance = entries[next].0;
                    let id = c.ui.scope_id().with(format!("maintenance-{next}"));
                    c.ui.ctx().memory_mut(|memory| memory.request_focus(id));
                }
            }
            y += 73.0 + 10.0;
        }
        let note = [
            Run::sans(
                "Your own files, your configuration in ",
                13.0,
                400.0,
                pal.text_muted,
            ),
            Run::mono("%APPDATA%\\nurt", 12.0, 400.0, pal.text_body),
            Run::sans(
                " and the journal of nurt are never touched by this setup.",
                13.0,
                400.0,
                pal.text_muted,
            ),
        ];
        y += 4.0;
        y + c.text(X, y, W, &note, 19.0)
    }

    // ---------------------------------------------------------------------------- removed

    fn screen_removed(&mut self, c: &mut Canvas<'_>) -> f32 {
        let pal = c.pal;
        let summary = self.summary.clone().unwrap_or_default();
        let count = summary.entries.saturating_sub(1);
        let lead = [Run::sans(
            format!("The journal entries 1 to {count} were undone."),
            15.0,
            400.0,
            pal.text_soft,
        )];
        let mut y = self.seal_heading(c, "Done", "Danaco Nurt was removed", &lead);
        let mut lines = vec![LogLine::command("undo", "setup")];
        for undone in &summary.undone {
            if undone.text == "close the setup journal" {
                continue;
            }
            lines.push(LogLine::answer(
                format!("undone {}  {}", undone.n, undone.text),
                Tone::Plain,
            ));
        }
        for warning in &summary.warnings {
            lines.push(LogLine::answer(format!("warning  {warning}"), Tone::Alert));
        }
        lines.push(LogLine::answer(
            "restored the state before the setup",
            Tone::Ok,
        ));
        let heights: f32 = lines
            .iter()
            .map(|line| c.measure(&widgets::log_runs(line, &pal), W - 30.0, 19.0).y)
            .sum();
        let height = 24.0 + 2.0 + heights;
        widgets::log_view(c, c.rect(X, y, W, height), &lines, false);
        y += height + 14.0;
        y + c.text(
            X,
            y,
            W,
            &[Run::sans(
                "Run this setup again at any time to install nurt again.",
                13.0,
                400.0,
                pal.text_muted,
            )],
            19.0,
        )
    }
}

/// A segment of a choice among a few values; returns that it was pressed.
fn segment(
    c: &mut Canvas<'_>,
    id: &str,
    rect: Rect,
    text: &str,
    selected: bool,
    group: &str,
) -> bool {
    let pal = c.pal;
    let response = c.interact(id, rect, Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::Role::RadioButton,
            true,
            selected,
            format!("{group}: {text}"),
        )
    });
    let hover = c.ease((id, "hover"), response.hovered(), 0.12);
    let border = if selected { pal.gold } else { pal.control_edge };
    let inner = if hover > 0.5 {
        pal.graphite_soft
    } else {
        pal.graphite
    };
    bordered_cut(&c.painter, rect, 8.0, 6.0, c.col(border), c.col(inner), 1.0);
    c.text_at(
        Align2::CENTER_CENTER,
        rect.center().x - c.origin.x,
        rect.center().y - c.origin.y - c.dy,
        &[Run::mono(
            text,
            12.0,
            if selected { 700.0 } else { 400.0 },
            if selected {
                pal.text_strong
            } else {
                pal.text_soft
            },
        )],
        16.0,
    );
    if c.live && c.ring && response.has_focus() {
        c.focus_ring(rect, 3.0, Some((8.0, 6.0)));
    }
    c.live && response.clicked()
}

/// The folder of `nurt.exe` for a folder that the user picked.
fn folder_from_picked(picked: &std::path::Path) -> String {
    let leaf = picked
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let path = if leaf == "bin" {
        picked.to_path_buf()
    } else if leaf == "nurt" {
        picked.join("bin")
    } else {
        picked.join("Nurt").join("bin")
    };
    path.display().to_string()
}

/// Title and the safe sentence of the error screen for a code of the engine.
fn error_texts(code: &str) -> (&'static str, &'static str) {
    match code {
        "payload-corrupt" | "payload-missing" => (
            "The package does not match its checksum",
            "The archive in this setup is not the one that was published, so it was not installed.",
        ),
        "access-denied" => (
            "Windows denied access to a folder",
            "Setup could not write where it was asked to.",
        ),
        "in-use" => (
            "A file of nurt is in use",
            "Close every terminal that runs nurt and try again.",
        ),
        "no-space" => (
            "There is not enough space on the disk",
            "The files did not fit on the disk that was chosen.",
        ),
        "invalid-path" => (
            "The folder is not valid",
            "Windows does not accept the folder that was chosen.",
        ),
        "elevation-declined" => (
            "Administrator approval was declined",
            "Windows did not allow the setup to change this PC for all users.",
        ),
        "elevation-failed" | "worker-exited" => (
            "The setup could not get administrator rights",
            "Windows did not start the part of the setup that needs them.",
        ),
        "registry" => (
            "Windows did not accept a registry change",
            "Setup could not write to the registry.",
        ),
        "journal-unreadable" => (
            "The journal of the setup cannot be read",
            "Without the journal the setup does not know what to undo.",
        ),
        _ => (
            "The setup could not finish",
            "Something stopped the setup before it was done.",
        ),
    }
}

fn requirement_text(id: &str, found: &str, required: &str) -> String {
    match id {
        "windows" => format!(
            "Windows 10 version 1809 or later is required (found {})",
            if found.is_empty() { "unknown" } else { found }
        ),
        "architecture" => format!("this setup is for {required}, this PC is {found}"),
        "space" => {
            let (free, need): (u64, u64) =
                (found.parse().unwrap_or(0), required.parse().unwrap_or(0));
            format!(
                "not enough free space: {} free, {} needed",
                megabytes(free),
                megabytes(need)
            )
        }
        _ => "this setup was built without the program archive".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_written_in_megabytes() {
        assert_eq!(megabytes(12_400_000), "13 MB");
        assert_eq!(megabytes(1), "1 MB");
    }

    #[test]
    fn a_picked_folder_becomes_the_folder_of_the_program() {
        assert_eq!(
            folder_from_picked(std::path::Path::new("D:\\Tools")),
            "D:\\Tools\\Nurt\\bin"
        );
        assert_eq!(
            folder_from_picked(std::path::Path::new("D:\\Tools\\Nurt")),
            "D:\\Tools\\Nurt\\bin"
        );
        assert_eq!(
            folder_from_picked(std::path::Path::new("D:\\Tools\\bin")),
            "D:\\Tools\\bin"
        );
    }

    #[test]
    fn the_log_is_plain_text() {
        assert_eq!(
            log_text(&LogLine::command("verify", "sha256")),
            "nurt> verify \"sha256\""
        );
        assert_eq!(log_text(&LogLine::answer("ok", Tone::Ok)), "  ok");
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
