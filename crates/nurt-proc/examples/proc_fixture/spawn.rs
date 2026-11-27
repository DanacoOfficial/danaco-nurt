// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-proc/examples/proc_fixture/spawn.rs
// Responsibility:   Modes for the tests of program resolution, arguments, environment and.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Modes for the tests of program resolution, arguments, environment and
//! input.
//!
//! | Mode | Arguments | Behaviour |
//! |---|---|---|
//! | `print-argv` | any | prints the arguments after the mode as a JSON array |
//! | `print-env` | — | prints the environment as a JSON object |
//! | `print-cwd` | — | prints the working directory |
//! | `count-input` | — | reads standard input to its end and prints the number of bytes |
//! | `exit-with` | code, optional milliseconds | sleeps, prints `exit <code>` and exits with the code |
//! | `write-file` | path, text | writes the text to the file and exits |

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::ExitCode;
use std::time::Duration;

/// Exit code for missing or malformed arguments.
const BAD_ARGUMENTS: u8 = 65;
/// Exit code when the mode cannot read or write what it needs.
const IO_FAILED: u8 = 74;

/// Runs `mode` when it belongs to this area.
pub(crate) fn run(mode: &str, args: &[String]) -> Option<ExitCode> {
    let code = match mode {
        "print-argv" => print(&serde_json::to_string(args).unwrap_or_default()),
        "print-env" => {
            let env: BTreeMap<String, String> = std::env::vars_os()
                .map(|(name, value)| {
                    (
                        name.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
                .collect();
            print(&serde_json::to_string(&env).unwrap_or_default())
        }
        "print-cwd" => match std::env::current_dir() {
            Ok(dir) => print(&dir.to_string_lossy()),
            Err(_) => ExitCode::from(IO_FAILED),
        },
        "count-input" => count_input(),
        "exit-with" => exit_with(args),
        "write-file" => write_file(args),
        _ => return None,
    };
    Some(code)
}

fn print(text: &str) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    match writeln!(stdout, "{text}").and_then(|()| stdout.flush()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(IO_FAILED),
    }
}

fn count_input() -> ExitCode {
    let mut buffer = Vec::new();
    match std::io::stdin().lock().read_to_end(&mut buffer) {
        Ok(read) => print(&read.to_string()),
        Err(_) => ExitCode::from(IO_FAILED),
    }
}

fn exit_with(args: &[String]) -> ExitCode {
    let Some(code) = args.first().and_then(|code| code.parse::<u8>().ok()) else {
        return ExitCode::from(BAD_ARGUMENTS);
    };
    if let Some(millis) = args.get(1).and_then(|millis| millis.parse::<u64>().ok()) {
        std::thread::sleep(Duration::from_millis(millis));
    }
    let _ = print(&format!("exit {code}"));
    ExitCode::from(code)
}

fn write_file(args: &[String]) -> ExitCode {
    let (Some(path), Some(text)) = (args.first(), args.get(1)) else {
        return ExitCode::from(BAD_ARGUMENTS);
    };
    match std::fs::write(path, text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(IO_FAILED),
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
