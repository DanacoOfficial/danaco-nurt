// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Command line program of the documentation generator.
//!
//! ```text
//! nurt-docgen check --root DIR [--allow-missing-pages] [--core-tokens N] [--total-tokens N]
//! nurt-docgen site --root DIR --out DIR [--version VERSION]
//! nurt-docgen man --root DIR --out DIR --date YYYY-MM-DD [--version VERSION]
//! nurt-docgen cheatsheets --root DIR
//! ```

// The program reports to the terminal; this is the output layer of the generator.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::ExitCode;

use nurt_docgen::catalog::Catalog;
use nurt_docgen::error::DocgenError;
use nurt_docgen::layout::Layout;
use nurt_docgen::manpages::{self, ManOptions};
use nurt_docgen::site;
use nurt_docgen::tasks::{self, CheckOptions};

const USAGE: &str = "usage: nurt-docgen <check|site|man|cheatsheets> --root DIR [options]";

#[derive(Default)]
struct Args {
    command: String,
    root: Option<PathBuf>,
    out: Option<PathBuf>,
    version: Option<String>,
    date: Option<String>,
    core_tokens: Option<u64>,
    total_tokens: Option<u64>,
    allow_missing_pages: bool,
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut iter = raw.iter();
    args.command = iter.next().cloned().ok_or_else(|| USAGE.to_string())?;
    while let Some(flag) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("option {name} needs a value"))
        };
        match flag.as_str() {
            "--root" => args.root = Some(PathBuf::from(value("--root")?)),
            "--out" => args.out = Some(PathBuf::from(value("--out")?)),
            "--version" => args.version = Some(value("--version")?),
            "--date" => args.date = Some(value("--date")?),
            "--core-tokens" => args.core_tokens = Some(number(&value("--core-tokens")?)?),
            "--total-tokens" => args.total_tokens = Some(number(&value("--total-tokens")?)?),
            "--allow-missing-pages" => args.allow_missing_pages = true,
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(args)
}

fn number(text: &str) -> Result<u64, String> {
    text.parse().map_err(|_| format!("{text} is not a number"))
}

fn require<T>(value: Option<T>, name: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("option {name} is required"))
}

fn run(args: &Args) -> Result<Vec<String>, String> {
    let layout = Layout::new(&require(args.root.clone(), "--root")?);
    let version = args
        .version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let failed = |error: DocgenError| error.to_string();
    match args.command.as_str() {
        "check" => {
            let options = CheckOptions {
                allow_missing_pages: args.allow_missing_pages,
                core_tokens: args.core_tokens,
                total_tokens: args.total_tokens,
            };
            tasks::check_repository(&layout, &options).map_err(failed)
        }
        "site" => {
            let out = require(args.out.clone(), "--out")?;
            let catalog = Catalog::load(&layout.catalog()).map_err(failed)?;
            let report = site::build(&layout, &catalog, &out, &version).map_err(failed)?;
            println!("{} pages written", report.pages);
            Ok(report
                .unresolved_links
                .iter()
                .map(|link| format!("unresolved link {link}"))
                .collect())
        }
        "man" => {
            let out = require(args.out.clone(), "--out")?;
            let date = require(args.date.clone(), "--date")?;
            let catalog = Catalog::load(&layout.catalog()).map_err(failed)?;
            let options = ManOptions {
                version: &version,
                date: &date,
            };
            let count = manpages::build(&layout, &catalog, &out, &options).map_err(failed)?;
            println!("{count} manual pages written");
            Ok(Vec::new())
        }
        "cheatsheets" => {
            for path in tasks::write_cheatsheets(&layout).map_err(failed)? {
                println!("{} written", path.display());
            }
            Ok(Vec::new())
        }
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(problems) if problems.is_empty() => ExitCode::SUCCESS,
        Ok(problems) => {
            for problem in &problems {
                eprintln!("{problem}");
            }
            eprintln!("{} problems found", problems.len());
            ExitCode::FAILURE
        }
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}
