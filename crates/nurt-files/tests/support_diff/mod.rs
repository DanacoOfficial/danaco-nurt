// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Test runner for differential tests against reference programs.
//!
//! The runner locates a program in `PATH`, runs it with a time limit and
//! normalizes its output so that it can be compared with the output of a Nurt
//! command. Reference programs are used by tests only.

// Each test target links a different subset of the runner.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use unicode_normalization::UnicodeNormalization;

/// Default time limit of one reference run.
pub(crate) const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Failure of a reference run or of output normalization.
#[derive(Debug)]
pub(crate) enum DiffError {
    /// The program could not be started or read.
    Io(io::Error),
    /// The program did not finish within the limit and was killed.
    Timeout(Duration),
    /// The output is not valid UTF-8.
    NotUtf8,
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "cannot run program: {e}"),
            Self::Timeout(d) => write!(f, "program exceeded the limit of {d:?}"),
            Self::NotUtf8 => f.write_str("output is not valid UTF-8"),
        }
    }
}

impl std::error::Error for DiffError {}

impl From<io::Error> for DiffError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Captured result of a reference run.
#[derive(Debug)]
pub(crate) struct Output {
    /// Exit code, absent when the process was ended by a signal.
    pub(crate) code: Option<i32>,
    /// Standard output.
    pub(crate) stdout: Vec<u8>,
    /// Standard error.
    pub(crate) stderr: Vec<u8>,
}

/// Finds `program` in `PATH`.
pub(crate) fn locate(program: &str) -> Option<PathBuf> {
    which::which(program).ok()
}

/// Variable that makes a missing reference program a failure instead of a
/// skipped test; a continuous integration run sets it to `1`, so that a test
/// that compares nothing cannot pass.
pub(crate) const REQUIRE_VARIABLE: &str = "NURT_REQUIRE_REFERENCE_PROGRAMS";

/// Finds `program` in `PATH` or reports that the test is skipped.
///
/// A test calls this first and returns when the result is `None`. With the
/// variable [`REQUIRE_VARIABLE`] set to `1` a missing program fails the test.
pub(crate) fn require(program: &str) -> Option<PathBuf> {
    let strict = std::env::var(REQUIRE_VARIABLE).is_ok_and(|value| value == "1");
    require_with(program, strict)
}

/// [`require`] with the strictness given.
// Skipped tests report through standard error because the test output is the only channel.
#[allow(clippy::print_stderr)]
pub(crate) fn require_with(program: &str, strict: bool) -> Option<PathBuf> {
    let found = locate(program);
    if found.is_none() {
        assert!(!strict, "program `{program}` not found in PATH");
        eprintln!("skipped: program `{program}` not found in PATH");
    }
    found
}

/// Runs `program` in `cwd` and waits at most `timeout`.
///
/// # Errors
///
/// Returns `DiffError::Timeout` after killing the process when the limit is
/// exceeded, and `DiffError::Io` when the process cannot be started.
pub(crate) fn run<S: AsRef<OsStr>>(
    program: &Path,
    args: &[S],
    cwd: &Path,
    timeout: Duration,
) -> Result<Output, DiffError> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            child.kill()?;
            child.wait()?;
            return Err(DiffError::Timeout(timeout));
        }
        thread::sleep(POLL_INTERVAL);
    };
    Ok(Output {
        code: status.code(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

fn drain<R: Read + Send + 'static>(stream: Option<R>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut stream) = stream {
            let _ = stream.read_to_end(&mut buffer);
        }
        buffer
    })
}

/// Decodes bytes as UTF-8. Line ends are kept as they are, so a difference
/// in line ends between Nurt and the reference program is a difference; the
/// line splitting of the callers drops one line end per line.
///
/// # Errors
///
/// Returns `DiffError::NotUtf8` when the bytes are not valid UTF-8.
pub(crate) fn normalize_text(bytes: &[u8]) -> Result<String, DiffError> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| DiffError::NotUtf8)
}

/// Whether `text` is a drive-letter or UNC path, whose separators are `\`.
fn is_windows_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.starts_with("\\\\")
        || (bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/'))
}

/// Converts one path to the form used by Nurt reports.
///
/// The path is made relative to `root`, separated by `/`, without a leading
/// `./`, and composed in Unicode normalization form C. A backslash is a
/// separator only in a Windows path (drive letter or UNC) or on Windows; in
/// any other path it is part of a file name, which a reference program and
/// Nurt must both report unchanged.
pub(crate) fn normalize_path(raw: &str, root: &Path) -> String {
    let windows = cfg!(windows) || is_windows_path(raw);
    let unified = if windows {
        raw.replace('\\', "/")
    } else {
        raw.to_owned()
    };
    let root_text = if windows {
        root.to_string_lossy().replace('\\', "/")
    } else {
        root.to_string_lossy().into_owned()
    };
    let prefix = format!("{}/", root_text.trim_end_matches('/'));
    let relative = unified.strip_prefix(&prefix).unwrap_or(&unified);
    let mut trimmed = relative;
    while let Some(rest) = trimmed.strip_prefix("./") {
        trimmed = rest;
    }
    trimmed.nfc().collect()
}

/// Splits program output into normalized paths, one per line, sorted.
///
/// # Errors
///
/// Returns `DiffError::NotUtf8` when the output is not valid UTF-8.
pub(crate) fn normalize_paths(bytes: &[u8], root: &Path) -> Result<Vec<String>, DiffError> {
    let text = normalize_text(bytes)?;
    let mut paths: Vec<String> = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| normalize_path(line, root))
        .collect();
    paths.sort();
    Ok(paths)
}

/// Splits NUL-terminated program output into normalized paths, sorted.
///
/// # Errors
///
/// Returns `DiffError::NotUtf8` when the output is not valid UTF-8.
pub(crate) fn normalize_nul_paths(bytes: &[u8], root: &Path) -> Result<Vec<String>, DiffError> {
    let text = normalize_text(bytes)?;
    let mut paths: Vec<String> = text
        .split('\0')
        .filter(|item| !item.is_empty())
        .map(|item| normalize_path(item, root))
        .collect();
    paths.sort();
    Ok(paths)
}

/// Difference between two sets of normalized lines.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SetDiff {
    /// Lines present only in the Nurt result.
    pub(crate) only_nurt: Vec<String>,
    /// Lines present only in the reference result.
    pub(crate) only_reference: Vec<String>,
}

impl SetDiff {
    /// Whether both results are equal.
    pub(crate) fn is_empty(&self) -> bool {
        self.only_nurt.is_empty() && self.only_reference.is_empty()
    }
}

/// Compares two lists as multisets of lines.
pub(crate) fn compare(nurt: &[String], reference: &[String]) -> SetDiff {
    let mut remaining = reference.to_vec();
    let mut only_nurt = Vec::new();
    for line in nurt {
        match remaining.iter().position(|r| r == line) {
            Some(index) => {
                remaining.swap_remove(index);
            }
            None => only_nurt.push(line.clone()),
        }
    }
    only_nurt.sort();
    remaining.sort();
    SetDiff {
        only_nurt,
        only_reference: remaining,
    }
}

/// Asserts that `nurt` and `reference` hold the same lines.
///
/// `label` names the compared command pair in the failure message.
pub(crate) fn assert_same(label: &str, nurt: &[String], reference: &[String]) {
    let diff = compare(nurt, reference);
    assert!(
        diff.is_empty(),
        "{label}: results differ\nonly in Nurt: {:?}\nonly in reference: {:?}",
        diff.only_nurt,
        diff.only_reference
    );
}

/// Runs `program` with the default limit and returns its output.
///
/// # Panics
///
/// Panics when the program cannot be run or exits with a code outside
/// `accepted`; the message carries the program name, the code and the standard
/// error text.
// The helper is called from test functions only; a failure aborts the test.
#[allow(clippy::expect_used)]
pub(crate) fn run_expect<S: AsRef<OsStr>>(
    program: &Path,
    args: &[S],
    cwd: &Path,
    accepted: &[i32],
) -> Output {
    let output = run(program, args, cwd, DEFAULT_TIMEOUT).expect("reference program runs");
    assert!(
        output.code.is_some_and(|code| accepted.contains(&code)),
        "{} exited with {:?}: {}",
        program.display(),
        output.code,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
