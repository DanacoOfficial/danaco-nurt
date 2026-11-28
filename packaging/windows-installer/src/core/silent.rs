// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/core/silent.rs
// Responsibility:   Silent mode: installation, repair and undo from the command line without a window, with exit codes for deployment scripts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Silent mode.
//!
//! Exit codes: 0 done, 1 failed (the changes were rolled back), 2 invalid arguments,
//! 3 requirements not met, 4 administrator approval declined.

use std::cell::RefCell;
use std::fs::File;
use std::io::Write;

use crate::app::{self, Startup, VERSION};
use crate::engine::{self, Env, UndoEnv};
use crate::model::{LogLine, Options, ProgressEvent, Scope, StageState};
use crate::{location, logfile, paths, requirements, sys};

/// Output of the silent mode: the console of the parent process and the log.
struct Output {
    console: Option<File>,
}

impl Output {
    fn say(&mut self, text: &str) {
        logfile::line(text);
        if let Some(console) = self.console.as_mut() {
            let _ = writeln!(console, "{text}");
        }
    }
}

/// The lines that a progress event adds to the output.
fn lines_of(event: &ProgressEvent) -> Vec<String> {
    let mut lines = Vec::new();
    for line in &event.log {
        match line {
            LogLine::Command { name, arg } => lines.push(
                format!("[{}] {name} {arg}", event.stage)
                    .trim_end()
                    .to_string(),
            ),
            LogLine::Answer { text, .. } => lines.push(format!("[{}]   {text}", event.stage)),
        }
    }
    if matches!(event.state, StageState::Failed) {
        lines.push(format!("[{}] failed", event.stage));
    }
    lines
}

/// Runs the silent mode and returns the exit code of the process.
pub(crate) fn run(startup: &Startup, arguments: &[String]) -> i32 {
    let mut output = Output {
        console: sys::parent_console(),
    };
    if let Some(version) = &startup.cli.version
        && version != VERSION
    {
        output.say(&format!("this setup installs {VERSION}, not {version}"));
        return 2;
    }
    if startup.cli.undo {
        undo(startup, arguments, &mut output)
    } else {
        install(startup, arguments, &mut output)
    }
}

fn elevate(arguments: &[String], output: &mut Output) -> i32 {
    output.say("administrator approval is needed: waiting for the elevation prompt");
    match app::relaunch_elevated(arguments, true) {
        Ok(Some(code)) => i32::try_from(code).unwrap_or(1),
        Ok(None) => 1,
        Err(error) => {
            output.say(&format!("elevation failed: {error}"));
            4
        }
    }
}

fn install(startup: &Startup, arguments: &[String], output: &mut Output) -> i32 {
    let mut base = Options::default();
    output.say(&format!("Danaco Nurt {VERSION} setup, silent mode"));
    if startup.cli.repair
        && let Some(existing) = &startup.existing
    {
        for path in engine::damaged_files(&existing.root) {
            output.say(&format!("repair: {path} differs from the journal"));
        }
        if let Some(recorded) = engine::recorded_options(&existing.root) {
            base = recorded;
        }
    }
    let options = crate::cli::options_of(&startup.cli, base);
    let rows = requirements::check(
        &paths::default_bin(options.scope, &startup.user),
        options.docs,
    );
    for row in &rows {
        output.say(&format!(
            "requirement {}: {} (found {}, required {})",
            row.id,
            if row.ok { "ok" } else { "fail" },
            row.found,
            row.required
        ));
    }
    if requirements::blocked(&rows) {
        output.say("the requirements are not met");
        return 3;
    }
    let report = location::check(
        &options,
        &startup.user,
        startup.elevated,
        requirements::required_space_for(options.docs),
    );
    if let Some(issue) = report.issue {
        output.say(&format!("invalid folder {}: {issue:?}", report.dir));
        return 2;
    }
    if options.scope == Scope::Machine && !startup.elevated {
        return elevate(arguments, output);
    }
    let lines = RefCell::new(Vec::<String>::new());
    let emit = |event: ProgressEvent| lines.borrow_mut().extend(lines_of(&event));
    let env = Env {
        options: &options,
        user: &startup.user,
        version: VERSION,
        emit: &emit,
        cancel: &|| false,
        pace: false,
        profile_runs: &crate::terminal::profile_runs,
    };
    let result = engine::install(&env);
    for line in lines.borrow().iter() {
        output.say(line);
    }
    match result {
        Ok(summary) => {
            for warning in &summary.warnings {
                output.say(&format!("warning: {warning}"));
            }
            output.say(&format!(
                "installed nurt {} to {}",
                summary.version, summary.bin
            ));
            0
        }
        Err(failure) => {
            output.say(&format!("failed in {}: {}", failure.stage, failure.error));
            output.say(if failure.rolled_back {
                "every change was rolled back"
            } else {
                "the rollback was incomplete; see the log"
            });
            1
        }
    }
}

fn undo(startup: &Startup, arguments: &[String], output: &mut Output) -> i32 {
    let Some(existing) = startup.existing.clone() else {
        output.say("Danaco Nurt is not installed");
        return 1;
    };
    if existing.scope == Scope::Machine && !startup.elevated {
        return elevate(arguments, output);
    }
    let lines = RefCell::new(Vec::<String>::new());
    let emit = |event: ProgressEvent| lines.borrow_mut().extend(lines_of(&event));
    let env = UndoEnv {
        root: &existing.root,
        user: &startup.user,
        remove_config: startup.cli.remove_config,
        emit: &emit,
        pace: false,
    };
    let result = engine::undo(&env);
    for line in lines.borrow().iter() {
        output.say(line);
    }
    if let Some(copy) = &startup.cli.from_temp {
        app::delete_after_exit(std::slice::from_ref(copy));
    }
    match result {
        Ok(summary) => {
            for warning in &summary.warnings {
                output.say(&format!("warning: {warning}"));
            }
            output.say(&format!(
                "removed Danaco Nurt: {} entries undone",
                summary.undone.len()
            ));
            i32::from(!summary.warnings.is_empty())
        }
        Err(error) => {
            output.say(&format!("the journal cannot be read: {error}"));
            1
        }
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
