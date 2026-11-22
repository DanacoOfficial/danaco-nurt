// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test program that imitates analyzers and formatters for the tests of the
//! analysis commands.
//!
//! The program behaves as a tool when it is started under the name of the
//! tool (a copy or a link named `prettier`, `gofmt`, `tsc` or `eslint`), so
//! the tests can place it where the detection of tools looks for programs.
//! `cargo test` builds it next to the test binaries.
//!
//! * `--version` prints a fixed version.
//! * Formatters (`prettier`, `gofmt`) list the files whose content differs from
//!   the normal form with `--list-different` or `-l` (exit code 1 when there
//!   are any) and rewrite them with `--write` or `-w`. The normal form has no
//!   trailing white space and ends with exactly one line feed.
//! * Reporters (`tsc`, `eslint`) print the file `<tool>.report` of the working
//!   directory and exit with code 1 when it is not empty.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Version printed by `--version`.
const VERSION: &str = "9.9.9";

/// Exit code of a run that found differences or diagnostics.
const FOUND: u8 = 1;

/// Exit code of a run with unusable arguments.
const USAGE: u8 = 2;

/// Exit code of a start under a name that is not an imitated tool.
const UNKNOWN_TOOL: u8 = 64;

/// Directory that formatters do not enter.
const SKIPPED_DIRECTORY: &str = "node_modules";

/// Tool the program imitates.
struct Imitated {
    name: String,
    list_flag: &'static str,
    write_flag: &'static str,
    extensions: &'static [&'static str],
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(name) = program_name() else {
        return ExitCode::from(UNKNOWN_TOOL);
    };
    if !matches!(name.as_str(), "prettier" | "gofmt" | "tsc" | "eslint") {
        return ExitCode::from(UNKNOWN_TOOL);
    }
    if args.first().is_some_and(|arg| arg == "--version") {
        write_out(&format!("{name} {VERSION}\n"));
        return ExitCode::SUCCESS;
    }
    match name.as_str() {
        "prettier" => format(
            &Imitated {
                name,
                list_flag: "--list-different",
                write_flag: "--write",
                extensions: &["ts", "tsx", "js", "jsx", "json", "css", "md"],
            },
            &args,
        ),
        "gofmt" => format(
            &Imitated {
                name,
                list_flag: "-l",
                write_flag: "-w",
                extensions: &["go"],
            },
            &args,
        ),
        _ => report(&name),
    }
}

fn write_out(text: &str) {
    let _ = std::io::stdout().write_all(text.as_bytes());
}

fn program_name() -> Option<String> {
    let program = std::env::args().next()?;
    Path::new(&program)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
}

fn report(name: &str) -> ExitCode {
    let text = fs::read_to_string(format!("{name}.report")).unwrap_or_default();
    write_out(&text);
    ExitCode::from(if text.trim().is_empty() { 0 } else { FOUND })
}

fn format(tool: &Imitated, args: &[String]) -> ExitCode {
    let list = args.iter().any(|arg| arg == tool.list_flag);
    let write = args.iter().any(|arg| arg == tool.write_flag);
    if list == write {
        let usage = format!(
            "{}: give {} or {}\n",
            tool.name, tool.list_flag, tool.write_flag
        );
        let _ = std::io::stderr().write_all(usage.as_bytes());
        return ExitCode::from(USAGE);
    }
    let mut files = Vec::new();
    for path in args.iter().filter(|arg| !arg.starts_with('-')) {
        collect(Path::new(path), tool.extensions, &mut files);
    }
    let mut different = false;
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let normal = normal_form(&text);
        if normal == text {
            continue;
        }
        different = true;
        if write {
            if fs::write(&file, normal).is_err() {
                return ExitCode::from(USAGE);
            }
        } else {
            write_out(&format!("{}\n", file.to_string_lossy().replace('\\', "/")));
        }
    }
    ExitCode::from(if list && different { FOUND } else { 0 })
}

fn collect(path: &Path, extensions: &[&str], found: &mut Vec<PathBuf>) {
    if path.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        let mut children: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        children.sort();
        for child in children {
            if child
                .file_name()
                .is_some_and(|name| name != SKIPPED_DIRECTORY)
            {
                collect(&child, extensions, found);
            }
        }
    } else if path
        .extension()
        .is_some_and(|ext| extensions.iter().any(|known| ext == *known))
    {
        let text = path.to_string_lossy();
        found.push(PathBuf::from(text.strip_prefix("./").unwrap_or(&text)));
    }
}

fn normal_form(text: &str) -> String {
    let mut normal = String::new();
    for line in text.lines() {
        normal.push_str(line.trim_end());
        normal.push('\n');
    }
    while normal.ends_with("\n\n") {
        normal.pop();
    }
    normal
}
