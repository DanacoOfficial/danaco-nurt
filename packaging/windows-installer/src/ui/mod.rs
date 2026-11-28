// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/ui/mod.rs
// Responsibility:   Interface of the wizard: a window drawn by egui in the design of the production identity, with the flows of the setup.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Interface of the wizard.
//!
//! The window is drawn, not assembled from system controls: the rail, the screens, the splash and
//! the dialogs follow `design/installer/` of the identity pack (960 x 600 points, Geist and Geist
//! Mono, the cut at the slope 3 : 4, the chevron as the only marker). The tree of UI Automation
//! comes from AccessKit through egui. The engine runs in the background (`app::Runner`); the
//! interface only follows its events.

#![allow(
    clippy::too_many_arguments,
    reason = "the components of the design take the parts of a control as separate values"
)]
#![allow(
    clippy::cognitive_complexity,
    reason = "a screen is one drawing routine at the coordinates of the design"
)]
#![allow(
    clippy::type_complexity,
    reason = "the table of the choices of a screen is written as it is drawn"
)]

mod chrome;
mod draw;
mod mark;
mod motion;
mod screens;
mod shape;
mod splash;
mod theme;
mod widgets;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, Rect, ViewportCommand, vec2};

use crate::app::{self, Runner, Startup};
use crate::engine;
use crate::model::{
    Existing, FinishEvent, LocationReport, LogLine, Options, Requirement, Scope, StageState,
    Summary, WorkerEvent,
};
use crate::{location, payload, requirements};
use draw::Canvas;
use mark::Art;
use theme::{Palette, WINDOW_H, WINDOW_W};

/// Exit code of the process, set by the interface.
static EXIT_CODE: AtomicI32 = AtomicI32::new(0);

/// A screen of the wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    /// Welcome and requirements.
    Welcome,
    /// Licence.
    Licence,
    /// Location and scope.
    Location,
    /// Options.
    Options,
    /// Review of the journal to be written.
    Review,
    /// Installing or removing.
    Install,
    /// The program is installed.
    Finish,
    /// The setup stopped.
    Error,
    /// Repair, Update or Undo.
    Maintenance,
    /// The program was removed.
    Removed,
}

/// The flow that the rail shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Welcome to Finish.
    Install,
    /// Maintenance, Undo, Done.
    Maintenance,
}

/// What a press of a button asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    /// The primary button.
    Primary,
    /// The secondary button.
    Secondary,
    /// The Cancel button.
    Cancel,
    /// The Close button of the caption.
    Close,
    /// The Minimize button of the caption.
    Minimize,
    /// The safe button of the dialog.
    DialogSafe,
    /// The other button of the dialog.
    DialogConfirm,
}

/// A dialog.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Dialog {
    /// Which question.
    pub(crate) kind: DialogKind,
    /// When it opened.
    pub(crate) opened: Instant,
}

/// Question of a dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogKind {
    /// Cancel the installation.
    CancelInstall,
    /// Stop the undo.
    CancelUndo,
    /// Close the setup on a screen that changed nothing.
    Close,
    /// Undo the setup that finished in this window.
    ConfirmUndo,
}

/// The buttons of the footer for a screen.
#[derive(Debug, Clone, Default)]
pub(crate) struct Footer {
    /// Label, enabled, shield.
    pub(crate) primary: Option<(String, bool, bool)>,
    /// Label, ghost.
    pub(crate) secondary: Option<(String, bool)>,
    /// Label.
    pub(crate) cancel: Option<String>,
}

/// What the maintenance screen asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Maintenance {
    /// Install the same version again, or update.
    Repair,
    /// Undo the setup.
    Undo,
}

/// What a run does, for the screen of progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunMode {
    /// Installs.
    Install,
    /// Removes.
    Undo,
}

/// The view of a run.
pub(crate) struct RunView {
    /// What the run does.
    pub(crate) mode: RunMode,
    /// Progress that the engine reported.
    pub(crate) target: f32,
    /// Progress that is shown, eased toward the target.
    pub(crate) shown: f32,
    /// Name of the stage that is shown.
    pub(crate) stage: String,
    /// The run is undoing its changes.
    pub(crate) rollback: bool,
    /// Lines of the log.
    pub(crate) log: Vec<LogLine>,
    /// The log as text, for the clipboard.
    pub(crate) text: Vec<String>,
    /// States of the stages.
    pub(crate) stages: HashMap<String, StageState>,
    /// When the end event arrived.
    pub(crate) ended: Option<Instant>,
    /// The end event.
    pub(crate) end: Option<FinishEvent>,
    /// The user asked to cancel.
    pub(crate) cancelling: bool,
}

impl RunView {
    fn new(mode: RunMode) -> Self {
        Self {
            mode,
            target: 0.0,
            shown: 0.0,
            stage: String::new(),
            rollback: false,
            log: Vec::new(),
            text: Vec::new(),
            stages: HashMap::new(),
            ended: None,
            end: None,
            cancelling: false,
        }
    }
}

/// The transition between two screens.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Transition {
    /// The screen that leaves.
    pub(crate) from: Screen,
    /// When the transition began.
    pub(crate) started: Instant,
}

/// The licence text and the progress of reading it.
pub(crate) struct Licence {
    /// LICENSE and NOTICE.
    pub(crate) text: String,
    /// The reader reached the end.
    pub(crate) read: bool,
    /// Offset of the text box.
    pub(crate) scroll: f32,
    /// Height of the text.
    pub(crate) content: f32,
}

/// State of the application.
pub(crate) struct App {
    pub(crate) startup: Startup,
    pub(crate) pal: Palette,
    pub(crate) art: Option<Art>,
    pub(crate) flow: Flow,
    pub(crate) index: usize,
    pub(crate) screen: Screen,
    pub(crate) transition: Option<Transition>,
    pub(crate) options: Options,
    pub(crate) licence: Licence,
    pub(crate) requirements: Vec<Requirement>,
    pub(crate) location: LocationReport,
    location_key: (String, Scope),
    pub(crate) required_space: u64,
    pub(crate) existing: Option<Existing>,
    pub(crate) maintenance: Maintenance,
    pub(crate) run: RunView,
    runner: Option<Runner>,
    pub(crate) summary: Option<Summary>,
    pub(crate) failure: Option<FinishEvent>,
    pub(crate) finish_checks: [bool; 2],
    pub(crate) dialog: Option<Dialog>,
    pub(crate) dialog_focus_pending: bool,
    pub(crate) pending: Option<Action>,
    pub(crate) scroll: f32,
    pub(crate) max_scroll: f32,
    pub(crate) keyboard: bool,
    pub(crate) copied: Option<Instant>,
    splash_started: Option<Instant>,
    splash_offset: f32,
    splash_done: bool,
    corners_set: bool,
    palette_checked: Instant,
    pub(crate) removed_from: Option<Existing>,
    ctx_handle: Option<egui::Context>,
}

/// Title of the window.
pub(crate) const TITLE: &str = "Danaco Nurt Setup";

impl App {
    fn new(startup: Startup, context: &egui::Context) -> Self {
        theme::install_fonts(context);
        let existing = startup
            .existing
            .clone()
            .filter(|existing| existing.journal_ok);
        // The documentation of the archive is offline HTML of some 200 files: the wizard offers it
        // as an option that the user switches on; the silent mode installs it unless `--no-docs`.
        let mut options = crate::cli::options_of(
            &startup.cli,
            Options {
                docs: false,
                ..Options::default()
            },
        );
        if let Some(recorded) = startup.resume.clone() {
            options = recorded;
        }
        let (flow, screen) = if existing.is_some() {
            (Flow::Maintenance, Screen::Maintenance)
        } else {
            (Flow::Install, Screen::Welcome)
        };
        let required_space = requirements::required_space_for(options.docs);
        let requirements = requirements::check(
            &crate::paths::default_bin(options.scope, &startup.user),
            options.docs,
        );
        let location = location::check(&options, &startup.user, startup.elevated, required_space);
        let maintenance = if startup.cli.undo {
            Maintenance::Undo
        } else {
            Maintenance::Repair
        };
        let licence_text = load_licence();
        let licence = Licence {
            read: licence_text.1,
            text: licence_text.0,
            scroll: 0.0,
            content: 0.0,
        };
        let splash_started = if startup.cli.no_animation {
            None
        } else {
            Some(Instant::now())
        };
        Self {
            pal: Palette::current(),
            art: None,
            flow,
            index: 0,
            screen,
            transition: None,
            location_key: (options.dir.clone(), options.scope),
            options,
            licence,
            requirements,
            location,
            required_space,
            existing,
            maintenance,
            run: RunView::new(RunMode::Install),
            runner: None,
            summary: None,
            failure: None,
            finish_checks: [true, false],
            dialog: None,
            dialog_focus_pending: false,
            pending: None,
            scroll: 0.0,
            max_scroll: 0.0,
            keyboard: false,
            copied: None,
            splash_started,
            splash_offset: 0.0,
            splash_done: splash_started.is_none(),
            corners_set: false,
            palette_checked: Instant::now(),
            removed_from: None,
            ctx_handle: None,
            startup,
        }
    }

    /// The steps of the rail for the flow.
    pub(crate) fn steps(&self) -> Vec<String> {
        let names: &[&str] = match self.flow {
            Flow::Install => &[
                "Welcome", "Licence", "Location", "Options", "Review", "Install", "Finish",
            ],
            Flow::Maintenance => &["Maintenance", "Undo", "Done"],
        };
        names.iter().map(|name| (*name).to_string()).collect()
    }

    /// The screens of the flow in order.
    fn order(flow: Flow) -> &'static [Screen] {
        match flow {
            Flow::Install => &[
                Screen::Welcome,
                Screen::Licence,
                Screen::Location,
                Screen::Options,
                Screen::Review,
                Screen::Install,
                Screen::Finish,
            ],
            Flow::Maintenance => &[Screen::Maintenance, Screen::Install, Screen::Removed],
        }
    }

    /// Opens a screen with the transition of the design.
    pub(crate) fn enter(&mut self, screen: Screen) {
        if screen != self.screen {
            self.transition = Some(Transition {
                from: self.screen,
                started: Instant::now(),
            });
        }
        self.screen = screen;
        self.scroll = 0.0;
        self.max_scroll = 0.0;
        if screen == Screen::Location {
            self.refresh_location(true);
        }
    }

    /// Moves in the flow by a step.
    pub(crate) fn go(&mut self, delta: i32) {
        let order = Self::order(self.flow);
        let next = self.index as i32 + delta;
        if next < 0 || next as usize >= order.len() {
            return;
        }
        self.index = next as usize;
        self.enter(order[self.index]);
        if self.screen == Screen::Install {
            self.begin_run();
        }
    }

    /// Checks the folder again when the choices changed.
    pub(crate) fn refresh_location(&mut self, force: bool) {
        let key = (self.options.dir.clone(), self.options.scope);
        if force || key != self.location_key {
            self.location_key = key;
            self.required_space = requirements::required_space_for(self.options.docs);
            self.location = location::check(
                &self.options,
                &self.startup.user,
                self.startup.elevated,
                self.required_space,
            );
        }
    }

    /// Starts the run that the current flow asks for.
    fn begin_run(&mut self) {
        let mode = if self.flow == Flow::Maintenance {
            RunMode::Undo
        } else {
            RunMode::Install
        };
        self.run = RunView::new(mode);
        self.failure = None;
        let context = self.context();
        let repaint = move || context.request_repaint();
        let user = self.startup.user.clone();
        self.runner = Some(match mode {
            RunMode::Install => app::start_install(self.options.clone(), user, repaint),
            RunMode::Undo => match self.existing.clone() {
                Some(existing) => {
                    self.removed_from = Some(existing.clone());
                    app::start_undo(&existing, self.startup.cli.remove_config, user, repaint)
                }
                None => app::start_undo(
                    &Existing {
                        version: String::new(),
                        root: Default::default(),
                        scope: Scope::User,
                        entries: 0,
                        journal_ok: false,
                    },
                    false,
                    user,
                    repaint,
                ),
            },
        });
    }

    fn context(&self) -> egui::Context {
        self.ctx_handle.clone().unwrap_or_default()
    }

    /// Takes the events of the run.
    fn poll(&mut self) {
        let Some(runner) = &self.runner else { return };
        let mut events = Vec::new();
        while let Ok(event) = runner.events.try_recv() {
            events.push(event);
        }
        for event in events {
            match event {
                WorkerEvent::Progress(progress) => {
                    self.run.target = progress.overall;
                    self.run.rollback = progress.state == StageState::Rollback;
                    self.run
                        .stages
                        .insert(progress.stage.clone(), progress.state);
                    self.run.stage =
                        screens::stage_label(self.run.mode, &progress.stage, progress.state);
                    for line in progress.log {
                        self.run.text.push(screens::log_text(&line));
                        self.run.log.push(line);
                    }
                }
                WorkerEvent::Finish(finish) => {
                    app::log_finish(&finish);
                    self.run.end = Some(finish);
                    self.run.ended = Some(Instant::now());
                }
            }
        }
    }

    /// Moves on when the run has ended and the bar has caught up.
    fn settle_run(&mut self, dt: f32) {
        let ease = 1.0 - (-dt * 9.0).exp();
        self.run.shown += (self.run.target - self.run.shown) * ease;
        let Some(end) = self.run.end.clone() else {
            return;
        };
        let Some(ended) = self.run.ended else { return };
        if end.ok {
            self.run.target = 1.0;
        }
        let caught_up = (self.run.target - self.run.shown).abs() < 0.004;
        if !caught_up || ended.elapsed() < Duration::from_millis(420) {
            return;
        }
        self.runner = None;
        self.run.ended = None;
        if end.ok {
            self.summary = end.summary.clone();
            if self.run.mode == RunMode::Install {
                self.existing = Some(Existing {
                    version: app::VERSION.to_string(),
                    root: end
                        .summary
                        .as_ref()
                        .map(|summary| summary.root.clone().into())
                        .unwrap_or_default(),
                    scope: self.options.scope,
                    entries: end
                        .summary
                        .as_ref()
                        .map_or(0, |summary| summary.entries as usize),
                    journal_ok: true,
                });
                self.index = Self::order(Flow::Install).len() - 1;
                self.enter(Screen::Finish);
            } else {
                self.existing = None;
                self.index = Self::order(Flow::Maintenance).len() - 1;
                self.enter(Screen::Removed);
            }
        } else if end.cancelled && end.rolled_back {
            if self.run.mode == RunMode::Install {
                self.index = 4;
                self.flow = Flow::Install;
                self.enter(Screen::Review);
            } else {
                self.existing = app::find_existing().filter(|existing| existing.journal_ok);
                self.index = 0;
                self.enter(if self.existing.is_some() {
                    Screen::Maintenance
                } else {
                    Screen::Removed
                });
            }
        } else {
            self.failure = Some(end);
            self.enter(Screen::Error);
        }
    }

    /// The buttons of the footer of the current screen.
    pub(crate) fn footer(&self) -> Footer {
        let label = |text: &str| Some(text.to_string());
        match self.screen {
            Screen::Welcome => Footer {
                primary: Some((
                    "Next".into(),
                    !requirements::blocked(&self.requirements),
                    false,
                )),
                secondary: None,
                cancel: label("Cancel"),
            },
            Screen::Licence => Footer {
                primary: Some(("Next".into(), self.licence.read, false)),
                secondary: Some(("Back".into(), false)),
                cancel: label("Cancel"),
            },
            Screen::Location => Footer {
                primary: Some(("Next".into(), self.location.issue.is_none(), false)),
                secondary: Some(("Back".into(), false)),
                cancel: label("Cancel"),
            },
            Screen::Options => Footer {
                primary: Some(("Next".into(), true, false)),
                secondary: Some(("Back".into(), false)),
                cancel: label("Cancel"),
            },
            Screen::Review => Footer {
                primary: Some(("Install".into(), true, self.options.scope == Scope::Machine)),
                secondary: Some(("Back".into(), false)),
                cancel: label("Cancel"),
            },
            Screen::Install => Footer {
                primary: None,
                secondary: None,
                cancel: label("Cancel"),
            },
            Screen::Finish => Footer {
                primary: Some(("Close".into(), true, false)),
                secondary: Some(("Undo this setup".into(), true)),
                cancel: None,
            },
            Screen::Error => Footer {
                primary: Some(("Try again".into(), true, false)),
                secondary: Some(("Copy the log".into(), false)),
                cancel: label("Close"),
            },
            Screen::Maintenance => {
                let primary = match (
                    self.maintenance,
                    self.existing
                        .as_ref()
                        .map(|existing| existing.version.as_str()),
                ) {
                    (Maintenance::Undo, _) => "Undo setup",
                    (_, Some(version)) if version != app::VERSION => "Update",
                    _ => "Repair",
                };
                Footer {
                    primary: Some((
                        primary.into(),
                        true,
                        self.existing
                            .as_ref()
                            .is_some_and(|existing| existing.scope == Scope::Machine),
                    )),
                    secondary: None,
                    cancel: label("Close"),
                }
            }
            Screen::Removed => Footer {
                primary: Some(("Close".into(), true, false)),
                secondary: None,
                cancel: None,
            },
        }
    }

    /// Runs the action of a press.
    fn act(&mut self, action: Action, context: &egui::Context) {
        if let Some(dialog) = self.dialog {
            match action {
                Action::DialogSafe => self.dialog = None,
                Action::DialogConfirm => {
                    self.dialog = None;
                    match dialog.kind {
                        DialogKind::Close => self.exit(context, 0),
                        DialogKind::ConfirmUndo => {
                            self.flow = Flow::Maintenance;
                            self.index = 0;
                            self.maintenance = Maintenance::Undo;
                            self.go(1);
                        }
                        DialogKind::CancelInstall | DialogKind::CancelUndo => {
                            self.run.cancelling = true;
                            if let Some(runner) = &self.runner {
                                runner.cancel();
                            }
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        match action {
            Action::Minimize => context.send_viewport_cmd(ViewportCommand::Minimized(true)),
            Action::Close => {
                if matches!(self.screen, Screen::Finish | Screen::Removed) {
                    self.finish_and_exit(context);
                } else {
                    self.ask_close();
                }
            }
            Action::Cancel => {
                if matches!(self.screen, Screen::Error | Screen::Maintenance) {
                    self.exit(context, i32::from(self.screen == Screen::Error));
                } else {
                    self.ask_close();
                }
            }
            Action::Primary => self.primary(context),
            Action::Secondary => self.secondary(context),
            Action::DialogSafe | Action::DialogConfirm => {}
        }
    }

    fn ask_close(&mut self) {
        let kind = match (self.screen, self.run.mode) {
            (Screen::Install, RunMode::Install) => DialogKind::CancelInstall,
            (Screen::Install, RunMode::Undo) => DialogKind::CancelUndo,
            _ => DialogKind::Close,
        };
        self.dialog = Some(Dialog {
            kind,
            opened: Instant::now(),
        });
        self.dialog_focus_pending = true;
    }

    fn primary(&mut self, context: &egui::Context) {
        let footer = self.footer();
        if footer
            .primary
            .as_ref()
            .is_none_or(|(_, enabled, _)| !enabled)
        {
            return;
        }
        match self.screen {
            Screen::Finish | Screen::Removed => self.finish_and_exit(context),
            Screen::Error => {
                self.flow = Flow::Install;
                self.index = 5;
                self.enter(Screen::Install);
                self.begin_run();
            }
            Screen::Maintenance => {
                if self.maintenance == Maintenance::Undo {
                    self.index = 0;
                    self.go(1);
                } else {
                    if let Some(recorded) = self
                        .existing
                        .as_ref()
                        .and_then(|existing| engine::recorded_options(&existing.root))
                    {
                        self.options = recorded;
                    }
                    self.flow = Flow::Install;
                    self.index = 5;
                    self.enter(Screen::Install);
                    self.begin_run();
                }
            }
            Screen::Install => {}
            _ => self.go(1),
        }
    }

    fn secondary(&mut self, context: &egui::Context) {
        match self.screen {
            Screen::Finish => {
                self.dialog = Some(Dialog {
                    kind: DialogKind::ConfirmUndo,
                    opened: Instant::now(),
                });
                self.dialog_focus_pending = true;
            }
            Screen::Error => {
                let text = self.run.text.join("\n");
                context.copy_text(text);
                self.copied = Some(Instant::now());
            }
            Screen::Licence | Screen::Location | Screen::Options | Screen::Review => self.go(-1),
            _ => {}
        }
    }

    /// Finishes the setup: runs the actions that the user ticked and closes the window.
    fn finish_and_exit(&mut self, context: &egui::Context) {
        if self.screen == Screen::Finish
            && let Some(summary) = &self.summary
        {
            let bin = std::path::PathBuf::from(&summary.bin);
            if self.finish_checks[0] {
                launch_terminal(&bin.join("nurt.exe"), self.options.terminal_profile);
            }
            if self.finish_checks[1] {
                app::launch_with_shell(
                    &std::path::PathBuf::from(&summary.root)
                        .join("doc")
                        .join("index.html"),
                );
            }
        }
        self.exit(context, 0);
    }

    fn exit(&mut self, context: &egui::Context, code: i32) {
        EXIT_CODE.store(code, Ordering::SeqCst);
        context.send_viewport_cmd(ViewportCommand::Close);
    }

    /// Time of the splash, or `None` when it is over.
    fn splash_time(&mut self) -> Option<f32> {
        let started = self.splash_started?;
        if self.splash_done {
            return None;
        }
        let real = started.elapsed().as_secs_f32();
        if !self.startup.animations {
            if real >= 0.7 {
                self.splash_done = true;
                return None;
            }
            return Some(splash::SKIP_TO);
        }
        let t = real + self.splash_offset;
        if t >= splash::END {
            self.splash_done = true;
            return None;
        }
        Some(t)
    }

    /// Skips the splash to the finished lockup.
    fn skip_splash(&mut self) {
        if let Some(started) = self.splash_started {
            let t = started.elapsed().as_secs_f32() + self.splash_offset;
            if (splash::SKIP_AFTER..splash::SKIP_TO).contains(&t) {
                self.splash_offset += splash::SKIP_TO - t;
            }
        }
    }

    /// Reads the keyboard and the pointer for the shortcuts of the wizard.
    fn read_input(&mut self, context: &egui::Context) {
        let (escape, enter, alt_n, alt_b, alt_c, tab, arrows, pointer, anything) =
            context.input(|input| {
                (
                    input.key_pressed(Key::Escape),
                    input.key_pressed(Key::Enter),
                    input.modifiers.alt && input.key_pressed(Key::N),
                    input.modifiers.alt && input.key_pressed(Key::B),
                    input.modifiers.alt && input.key_pressed(Key::C),
                    input.key_pressed(Key::Tab),
                    input.key_pressed(Key::ArrowDown) || input.key_pressed(Key::ArrowUp),
                    input.pointer.any_pressed(),
                    input.events.iter().any(|event| {
                        matches!(
                            event,
                            egui::Event::Key { pressed: true, .. }
                                | egui::Event::PointerButton { pressed: true, .. }
                        )
                    }),
                )
            });
        if self.splash_started.is_some() && !self.splash_done {
            if anything {
                self.skip_splash();
            }
            return;
        }
        if tab || arrows {
            self.keyboard = true;
        }
        if pointer {
            self.keyboard = false;
        }
        if self.pending.is_some() {
            return;
        }
        if self.dialog.is_some() {
            if escape {
                self.pending = Some(Action::DialogSafe);
            }
            return;
        }
        let footer = self.footer();
        let focused_text =
            context.memory(|memory| memory.focused().is_some()) && self.screen == Screen::Location;
        let focus_free = context.memory(|memory| memory.focused().is_none());
        if escape && footer.cancel.is_some() {
            self.pending = Some(Action::Cancel);
        } else if alt_n && footer.primary.is_some() {
            self.pending = Some(Action::Primary);
        } else if alt_b
            && footer
                .secondary
                .as_ref()
                .is_some_and(|(label, _)| label == "Back")
        {
            self.pending = Some(Action::Secondary);
        } else if alt_c && footer.cancel.is_some() {
            self.pending = Some(Action::Cancel);
        } else if enter && (focus_free || focused_text) && footer.primary.is_some() {
            self.pending = Some(Action::Primary);
        }
    }
}

/// Opens a terminal with `nurt.exe`: a tab of Windows Terminal with the profile of the program
/// when the profile was chosen, otherwise a console window.
///
/// The tab gets `exe` as its command line besides the profile. A Windows Terminal that was open
/// before the setup wrote the fragment does not know the profile yet and opens its default one,
/// so the profile alone would start another shell. When Windows Terminal cannot be started, or
/// the path holds `;`, which Windows Terminal reads as the separator of its commands, the console
/// window is opened instead.
fn launch_terminal(exe: &std::path::Path, profile: bool) {
    use std::os::windows::process::CommandExt;
    let user = crate::model::UserPaths::current();
    let separator = crate::shortcuts::TERMINAL_COMMAND_SEPARATOR;
    let in_terminal = profile
        && !exe.to_string_lossy().contains(separator)
        && crate::paths::terminal_installed(&user)
        && std::process::Command::new(crate::paths::terminal_executable(&user))
            .args(["-p", crate::textedit::PROFILE_NAME])
            .arg(exe)
            .spawn()
            .is_ok();
    if !in_terminal {
        let mut command = std::process::Command::new("cmd.exe");
        command.args(["/C", "start", "Danaco Nurt"]).arg(exe);
        command.creation_flags(0x0800_0000);
        let _ = command.spawn();
    }
}

/// The text of LICENSE and NOTICE of the archive and whether it could be read.
fn load_licence() -> (String, bool) {
    match (payload::read_text("LICENSE"), payload::read_text("NOTICE")) {
        (Ok(license), Ok(notice)) => (format!("{}\n\n{}\n\n{}", license.trim_end(), "-".repeat(60), notice.trim_end()), false),
        _ => ("The text of the licence could not be read from the package.\nThe licence of Danaco Nurt is the Apache License, Version 2.0: https://www.apache.org/licenses/LICENSE-2.0\n".to_string(), true),
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let [r, g, b, _] = self.pal.black_deep.to_array();
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            1.0,
        ]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        if self.ctx_handle.is_none() {
            self.ctx_handle = Some(context.clone());
        }
        if self
            .art
            .as_ref()
            .is_none_or(|art| (art.scale - context.pixels_per_point()).abs() > 0.01)
        {
            self.art = Some(Art::new(&context, context.pixels_per_point()));
        }
        if !self.corners_set {
            self.corners_set = true;
            crate::sys::round_window_corners(TITLE);
        }
        if self.palette_checked.elapsed() > Duration::from_secs(2) {
            self.pal = Palette::current();
            self.palette_checked = Instant::now();
        }
        let dt = context.input(|input| input.stable_dt).min(0.1);
        self.read_input(&context);
        self.poll();
        if self.screen == Screen::Install {
            self.settle_run(dt);
        }
        let origin = ui.max_rect().min;
        let splash_time = self.splash_time();
        let animating = splash_time.is_some()
            || self.transition.is_some()
            || self.screen == Screen::Install
            || self.dialog.is_some()
            || self.copied.is_some();

        // The wizard is drawn first; the splash is drawn over it while it fades out.
        let wizard_alpha = splash_time.map_or(1.0, |t| {
            if self.startup.animations {
                splash::wizard_alpha(t)
            } else {
                0.0
            }
        });
        if wizard_alpha > 0.0 {
            self.draw_wizard(ui, origin, wizard_alpha, splash_time.is_none());
        }
        if let Some(t) = splash_time {
            let alpha = if self.startup.animations {
                splash::splash_alpha(t)
            } else {
                1.0
            };
            if alpha > 0.0
                && let Some(art) = self.art.take()
            {
                let painter = ui.painter().clone();
                {
                    let mut canvas = Canvas {
                        ui,
                        painter,
                        origin,
                        pal: self.pal,
                        alpha,
                        dy: 0.0,
                        ring: false,
                        live: false,
                    };
                    splash::draw(&mut canvas, &art, t, app::VERSION);
                }
                self.art = Some(art);
            }
        }
        if let Some(action) = self.pending.take() {
            self.act(action, &context);
        }
        if self
            .transition
            .is_some_and(|transition| transition.started.elapsed() > Duration::from_millis(340))
        {
            self.transition = None;
        }
        if self
            .copied
            .is_some_and(|at| at.elapsed() > Duration::from_secs(2))
        {
            self.copied = None;
        }
        if animating {
            context.request_repaint();
        } else {
            context.request_repaint_after(Duration::from_millis(500));
        }
    }
}

impl App {
    fn draw_wizard(
        &mut self,
        ui: &mut egui::Ui,
        origin: egui::Pos2,
        alpha: f32,
        interactive: bool,
    ) {
        let painter = ui.painter().clone();
        let live = interactive && self.dialog.is_none();
        let ring = self.keyboard;
        let transition = self.transition;
        let pal = self.pal;
        let body = chrome::body_rect().translate(origin.to_vec2());
        {
            let canvas = Canvas {
                ui,
                painter: painter.clone(),
                origin,
                pal,
                alpha,
                dy: 0.0,
                ring,
                live,
            };
            canvas.painter.rect_filled(
                Rect::from_min_size(origin, vec2(WINDOW_W, WINDOW_H)),
                0.0,
                canvas.col(pal.graphite),
            );
        }
        // The screens, clipped to the body area; the one that leaves fades out while the new one fades in.
        let clipped = painter.with_clip_rect(body);
        if let Some(transition) = transition {
            let k = if self.startup.animations && !pal.high_contrast {
                motion::interface(motion::norm(
                    transition.started.elapsed().as_secs_f32(),
                    0.0,
                    theme::MOTION_SLOW,
                ))
            } else {
                1.0
            };
            {
                let mut canvas = Canvas {
                    ui,
                    painter: clipped.clone(),
                    origin,
                    pal,
                    alpha: alpha * (1.0 - k),
                    dy: 8.0 * k,
                    ring: false,
                    live: false,
                };
                self.draw_screen(&mut canvas, transition.from, false);
            }
            let mut canvas = Canvas {
                ui,
                painter: clipped,
                origin,
                pal,
                alpha: alpha * k,
                dy: 8.0 * (1.0 - k) - self.scroll,
                ring,
                live,
            };
            self.draw_screen(&mut canvas, self.screen, true);
        } else {
            let mut canvas = Canvas {
                ui,
                painter: clipped,
                origin,
                pal,
                alpha,
                dy: -self.scroll,
                ring,
                live,
            };
            self.draw_screen(&mut canvas, self.screen, true);
        }
        // The scroll of the wheel.
        if live {
            let delta = ui.input(|input| input.smooth_scroll_delta.y);
            if delta != 0.0
                && self.max_scroll > 0.0
                && ui
                    .input(|input| input.pointer.hover_pos())
                    .is_some_and(|pos| body.contains(pos))
            {
                self.scroll = (self.scroll - delta).clamp(0.0, self.max_scroll);
            }
        }
        {
            let mut canvas = Canvas {
                ui,
                painter: painter.clone(),
                origin,
                pal,
                alpha,
                dy: 0.0,
                ring,
                live,
            };
            canvas.painter.rect_filled(
                canvas.rect(
                    theme::RAIL_W,
                    theme::WINDOW_H - theme::FOOTER_H,
                    theme::WINDOW_W - theme::RAIL_W,
                    theme::FOOTER_H,
                ),
                0.0,
                canvas.col(pal.graphite),
            );
            canvas.painter.rect_filled(
                canvas.rect(
                    theme::RAIL_W,
                    0.0,
                    theme::WINDOW_W - theme::RAIL_W,
                    theme::CAPTION_H,
                ),
                0.0,
                canvas.col(pal.graphite),
            );
            self.draw_footer(&mut canvas);
            self.draw_caption(&mut canvas);
            self.draw_rail(&mut canvas);
        }
        if let Some(dialog) = self.dialog {
            let mut canvas = Canvas {
                ui,
                painter: painter.clone(),
                origin,
                pal,
                alpha,
                dy: 0.0,
                ring,
                live: interactive,
            };
            self.draw_dialog(&mut canvas, &dialog);
        }
        // The border of the window.
        painter.rect_stroke(
            Rect::from_min_size(origin, vec2(WINDOW_W, WINDOW_H)),
            0.0,
            egui::Stroke::new(1.0, pal.line.gamma_multiply(alpha)),
            egui::StrokeKind::Inside,
        );
    }
}

/// Opens the window and runs the wizard; returns the exit code of the process.
pub(crate) fn run(startup: Startup) -> i32 {
    let from_temp = startup.cli.from_temp.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(TITLE)
            .with_inner_size([WINDOW_W, WINDOW_H])
            .with_min_inner_size([WINDOW_W, WINDOW_H])
            .with_max_inner_size([WINDOW_W, WINDOW_H])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_decorations(false),
        centered: true,
        multisampling: 0,
        ..eframe::NativeOptions::default()
    };
    let result = eframe::run_native(
        TITLE,
        options,
        Box::new(|creation| {
            let mut app = App::new(startup, &creation.egui_ctx);
            app.ctx_handle = Some(creation.egui_ctx.clone());
            Ok(Box::new(app))
        }),
    );
    if let Some(copy) = from_temp {
        app::delete_after_exit(&[copy]);
    }
    match result {
        Ok(()) => EXIT_CODE.load(Ordering::SeqCst),
        Err(error) => {
            crate::logfile::line(&format!("window failed: {error}"));
            crate::sys::error_box(
                "Danaco Nurt",
                "The window of the setup cannot start: the graphics of this PC are not available to it.\n\nUse the silent mode instead: setup.exe /S",
            );
            3
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
