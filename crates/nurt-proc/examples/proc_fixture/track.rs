// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Modes for the tests of change detection.
//!
//! `track-mutate` applies file operations given as words: `write PATH TEXT`,
//! `append PATH TEXT`, `remove PATH`, `rename FROM TO` and `mkdir PATH`.

use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

/// Exit code for an operation that failed or a malformed operation list.
const OPERATION_FAILED: u8 = 3;

/// Runs `mode` when it belongs to this area.
pub(crate) fn run(mode: &str, args: &[String]) -> Option<ExitCode> {
    (mode == "track-mutate").then(|| match mutate(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(OPERATION_FAILED),
    })
}

fn malformed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "malformed operation list")
}

fn mutate(args: &[String]) -> io::Result<()> {
    let mut words = args.iter();
    while let Some(operation) = words.next() {
        let mut operand = || words.next().ok_or_else(malformed);
        match operation.as_str() {
            "write" => {
                let path = operand()?;
                let text = operand()?;
                if let Some(parent) = std::path::Path::new(path).parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, text)?;
            }
            "append" => {
                let path = operand()?;
                let text = operand()?;
                fs::OpenOptions::new()
                    .append(true)
                    .open(path)?
                    .write_all(text.as_bytes())?;
            }
            "remove" => fs::remove_file(operand()?)?,
            "rename" => {
                let from = operand()?;
                let to = operand()?;
                fs::rename(from, to)?;
            }
            "mkdir" => fs::create_dir_all(operand()?)?,
            _ => return Err(malformed()),
        }
    }
    Ok(())
}
