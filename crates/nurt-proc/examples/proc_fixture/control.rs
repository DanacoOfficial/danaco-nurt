// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Modes for the tests of process trees, limits, signals and suspension.
//!
//! | Mode | Arguments | Behaviour |
//! |---|---|---|
//! | `spawn-tree` | depth, milliseconds | prints `pid <id>`, starts the same mode with depth minus one, sleeps |
//! | `detach` | milliseconds, `keep` or `close` | starts `silent-sleep` with the output kept open or closed, prints `pid <id>` of it, ends |
//! | `ignore-term` | milliseconds | blocks the termination signal, prints `ready`, sleeps |
//! | `burn-cpu` | milliseconds | uses the processor without output |
//! | `alloc` | mebibytes, milliseconds | allocates and touches the memory, prints `allocated`, sleeps |
//! | `silent-sleep` | milliseconds | sleeps without output |
//! | `print-every` | milliseconds, count | prints `line <n>` at the interval |
//! | `copy-input` | none | copies standard input to standard output |
//! | `print-both` | none | prints `out` to standard output and `err` to standard error |
//! | `raise` | signal number | sends the signal to itself (Unix) |

use std::io::{Read, Write};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

/// Exit code for missing or malformed arguments.
const BAD_ARGUMENTS: u8 = 65;
/// Exit code when a child process cannot be started.
const CHILD_FAILED: u8 = 66;
/// Bytes in one mebibyte.
const MEBIBYTE: usize = 1024 * 1024;

/// Runs `mode` when it belongs to this area.
pub(crate) fn run(mode: &str, args: &[String]) -> Option<ExitCode> {
    let code = match mode {
        "spawn-tree" => spawn_tree(args),
        "detach" => detach(args),
        "ignore-term" => ignore_term(args),
        "burn-cpu" => burn_cpu(args),
        "alloc" => alloc(args),
        "silent-sleep" => number(args, 0).map(sleep),
        "print-every" => print_every(args),
        "copy-input" => Some(copy_input()),
        "print-both" => Some(print_both()),
        "raise" => raise(args),
        _ => return None,
    };
    Some(code.unwrap_or(ExitCode::from(BAD_ARGUMENTS)))
}

fn number(args: &[String], index: usize) -> Option<u64> {
    args.get(index)?.parse().ok()
}

fn sleep(millis: u64) -> ExitCode {
    std::thread::sleep(Duration::from_millis(millis));
    ExitCode::SUCCESS
}

fn say(line: &str) {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{line}").ok();
    stdout.flush().ok();
}

fn spawn_tree(args: &[String]) -> Option<ExitCode> {
    let depth = number(args, 0)?;
    let millis = number(args, 1)?;
    say(&format!("pid {}", std::process::id()));
    if depth > 0 {
        let started = Command::new(std::env::current_exe().ok()?)
            .args(["spawn-tree", &(depth - 1).to_string(), &millis.to_string()])
            .spawn()
            .map(|child| child.id());
        if started.is_err() {
            return Some(ExitCode::from(CHILD_FAILED));
        }
    }
    Some(sleep(millis))
}

fn detach(args: &[String]) -> Option<ExitCode> {
    let millis = number(args, 0)?;
    let keep = match args.get(1)?.as_str() {
        "keep" => true,
        "close" => false,
        _ => return None,
    };
    let mut command = Command::new(std::env::current_exe().ok()?);
    command.args(["silent-sleep", &millis.to_string()]);
    if !keep {
        command.stdout(Stdio::null()).stderr(Stdio::null());
        release_inherited_output();
    }
    match command.spawn().map(|child| child.id()) {
        Ok(pid) => {
            say(&format!("pid {pid}"));
            Some(ExitCode::SUCCESS)
        }
        Err(_) => Some(ExitCode::from(CHILD_FAILED)),
    }
}

/// Keeps the output handles of this process out of the child: on Windows a
/// child inherits every inheritable handle, so the pipe of the run would stay
/// open in it even with its own standard handles replaced.
#[cfg(windows)]
#[allow(unsafe_code)] // Win32 handle calls have no safe wrapper
fn release_inherited_output() {
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for which in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: plain calls on the standard handles of this process; a
        // failure leaves the handle as it is.
        unsafe {
            let handle = GetStdHandle(which);
            SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0);
        }
    }
}

#[cfg(not(windows))]
fn release_inherited_output() {}

fn ignore_term(args: &[String]) -> Option<ExitCode> {
    let millis = number(args, 0)?;
    #[cfg(unix)]
    {
        use nix::sys::signal::{SigSet, Signal};
        let mut blocked = SigSet::empty();
        blocked.add(Signal::SIGTERM);
        blocked.thread_block().ok()?;
    }
    say("ready");
    Some(sleep(millis))
}

fn burn_cpu(args: &[String]) -> Option<ExitCode> {
    let duration = Duration::from_millis(number(args, 0)?);
    let start = Instant::now();
    let mut counter: u64 = 0;
    while start.elapsed() < duration {
        counter = std::hint::black_box(counter.wrapping_add(1));
    }
    Some(ExitCode::SUCCESS)
}

fn alloc(args: &[String]) -> Option<ExitCode> {
    let mebibytes = usize::try_from(number(args, 0)?).ok()?;
    let millis = number(args, 1)?;
    let mut blocks: Vec<Vec<u8>> = Vec::with_capacity(mebibytes);
    for _ in 0..mebibytes {
        blocks.push(std::hint::black_box(vec![1u8; MEBIBYTE]));
    }
    say("allocated");
    let code = sleep(millis);
    drop(std::hint::black_box(blocks));
    Some(code)
}

fn print_every(args: &[String]) -> Option<ExitCode> {
    let millis = number(args, 0)?;
    let count = number(args, 1)?;
    for line in 0..count {
        say(&format!("line {line}"));
        std::thread::sleep(Duration::from_millis(millis));
    }
    Some(ExitCode::SUCCESS)
}

fn copy_input() -> ExitCode {
    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_err() {
        return ExitCode::FAILURE;
    }
    let mut stdout = std::io::stdout().lock();
    if stdout
        .write_all(&input)
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn print_both() -> ExitCode {
    say("out");
    let mut stderr = std::io::stderr().lock();
    writeln!(stderr, "err").ok();
    ExitCode::SUCCESS
}

#[cfg(unix)]
fn raise(args: &[String]) -> Option<ExitCode> {
    let number = i32::try_from(number(args, 0)?).ok()?;
    let signal = nix::sys::signal::Signal::try_from(number).ok()?;
    nix::sys::signal::raise(signal).ok()?;
    Some(ExitCode::FAILURE)
}

#[cfg(not(unix))]
fn raise(_args: &[String]) -> Option<ExitCode> {
    None
}
