// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           packaging/windows-installer/src/main.rs
// Responsibility:   Entry point of the setup wizard of Danaco Nurt for Windows.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.2, 2026-11-28
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Setup wizard of Danaco Nurt for Windows.
//!
//! One executable that carries the program archive. Without arguments it opens the wizard,
//! a window drawn by egui (`ui/`); with `/S` it installs without a window. Copied to
//! `uninstall.exe` it undoes the setup. `packaging/windows-installer/README.md` describes the
//! steps, the switches, the journal and the build.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
#[path = "core/app.rs"]
mod app;
#[cfg(windows)]
#[path = "core/cli.rs"]
mod cli;
#[cfg(windows)]
#[path = "core/engine.rs"]
mod engine;
#[cfg(windows)]
#[path = "core/error.rs"]
mod error;
#[cfg(windows)]
#[path = "core/journal.rs"]
mod journal;
#[cfg(windows)]
#[path = "core/location.rs"]
mod location;
#[cfg(windows)]
#[path = "core/logfile.rs"]
mod logfile;
#[cfg(windows)]
#[path = "core/model.rs"]
mod model;
#[cfg(windows)]
#[path = "core/paths.rs"]
mod paths;
#[cfg(windows)]
#[path = "core/payload.rs"]
mod payload;
#[cfg(windows)]
#[path = "core/registry.rs"]
mod registry;
#[cfg(windows)]
#[path = "core/requirements.rs"]
mod requirements;
#[cfg(windows)]
mod shortcuts;
#[cfg(windows)]
#[path = "core/silent.rs"]
mod silent;
#[cfg(windows)]
mod sys;
#[cfg(windows)]
mod terminal;
#[cfg(windows)]
mod textedit;
#[cfg(windows)]
mod ui;

#[cfg(windows)]
fn print_or_box(text: &str, quiet: bool) {
    use std::io::Write;
    match sys::parent_console() {
        Some(mut console) => {
            let _ = write!(console, "{text}");
        }
        None if quiet => logfile::line(text),
        None => sys::error_box("Danaco Nurt Setup", text),
    }
}

#[cfg(windows)]
fn main() {
    let arguments = app::own_arguments();
    let cli = match cli::parse(&arguments) {
        Ok(cli) => cli,
        Err(message) => {
            let quiet = arguments.iter().any(|argument| {
                argument.eq_ignore_ascii_case("/s") || argument.eq_ignore_ascii_case("--silent")
            });
            print_or_box(&format!("{message}\n\n{}", cli::USAGE), quiet);
            std::process::exit(2);
        }
    };
    logfile::open(cli.log.clone());
    let quiet_panic = cli.silent || cli.worker.is_some();
    std::panic::set_hook(Box::new(move |info| {
        logfile::line(&format!("panic: {info}"));
        if !quiet_panic {
            sys::error_box(
                "Danaco Nurt Setup",
                "The setup stopped because of an unexpected error. The details are in the log file nurt-setup.log in the temporary folder of this account.\n\nThe silent mode (setup.exe /S) does not need the window.",
            );
        }
    }));
    logfile::line(&format!("start {} {}", app::VERSION, arguments.join(" ")));
    if cli.help {
        print_or_box(cli::USAGE, cli.silent);
        std::process::exit(0);
    }
    if let Some(job) = cli.worker.clone() {
        std::process::exit(app::worker(&job));
    }

    let mut startup = app::startup(cli);
    let named_uninstall = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.file_stem()
                .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        })
        .is_some_and(|stem| stem == "uninstall");
    if named_uninstall {
        startup.cli.undo = true;
    }
    // A setup that runs from the root of the installation cannot replace or delete itself.
    if let Some(existing) = startup.existing.clone()
        && startup.cli.from_temp.is_none()
        && app::runs_from_installation(&existing)
    {
        let code = match app::relaunch_from_temp(&arguments, startup.cli.silent) {
            Ok(code) => code.unwrap_or(0),
            Err(error) => {
                logfile::line(&format!("relaunch failed: {error}"));
                sys::error_box("Danaco Nurt", &error.to_string());
                1
            }
        };
        std::process::exit(code);
    }
    if startup.cli.silent {
        std::process::exit(silent::run(&startup, &arguments));
    }
    std::process::exit(ui::run(startup));
}

#[cfg(not(windows))]
fn main() {
    std::process::exit(1);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
