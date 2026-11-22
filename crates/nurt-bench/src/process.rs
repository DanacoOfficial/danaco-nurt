// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Child processes with a time limit and captured output.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::error::{BenchError, Result};

/// Interval between checks whether a child has exited.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Time to collect output after the child has exited. Background processes
/// started by the child may keep its output open; their later output is not
/// collected.
const OUTPUT_GRACE: Duration = Duration::from_millis(200);

/// Size of one read from a pipe of the child.
const READ_CHUNK: usize = 8192;

/// Process to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSpec {
    /// Program name or path.
    pub program: String,
    /// Arguments.
    pub args: Vec<String>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Environment variables set in addition to the inherited environment.
    pub env: Vec<(String, String)>,
    /// Data written to standard input; without it standard input is empty.
    pub stdin: Option<Vec<u8>>,
    /// Time limit; the process and its process group are killed when it is
    /// exceeded.
    pub timeout: Duration,
}

/// Result of a finished process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    /// Exit code; absent when the process was killed by a signal or the time
    /// limit.
    pub code: Option<i32>,
    /// Captured standard output.
    pub stdout: Vec<u8>,
    /// Captured standard error.
    pub stderr: Vec<u8>,
    /// Whether the time limit was exceeded.
    pub timed_out: bool,
    /// Wall clock time of the process.
    pub duration: Duration,
}

impl ProcessOutput {
    /// Whether the process exited with code 0.
    #[must_use]
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

impl ProcessSpec {
    /// Creates a specification from a program and its arguments.
    #[must_use]
    pub fn new(
        program: impl Into<String>,
        args: Vec<String>,
        cwd: PathBuf,
        timeout: Duration,
    ) -> Self {
        Self {
            program: program.into(),
            args,
            cwd,
            env: Vec::new(),
            stdin: None,
            timeout,
        }
    }

    /// Creates a [`Command`] for the specification. On Unix the child leads
    /// a new process group, so that the whole group can be killed.
    #[must_use]
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args).current_dir(&self.cwd);
        for (name, value) in &self.env {
            command.env(name, value);
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
        }
        command
    }
}

/// Runs a process to its end or to its time limit.
///
/// # Errors
///
/// Returns [`BenchError::Process`] when the process cannot be started or
/// waited for.
pub fn run(spec: &ProcessSpec) -> Result<ProcessOutput> {
    let started = Instant::now();
    let mut command = spec.command();
    command
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| BenchError::Process(format!("cannot start {}: {error}", spec.program)))?;
    if let (Some(data), Some(mut stdin)) = (spec.stdin.clone(), child.stdin.take()) {
        thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    let stdout = child.stdout.take().map(spawn_reader);
    let stderr = child.stderr.take().map(spawn_reader);
    let (code, timed_out) = wait(&mut child, spec.timeout)?;
    let deadline = Instant::now() + OUTPUT_GRACE;
    Ok(ProcessOutput {
        code,
        stdout: stdout
            .map(|reader| collect(&reader, deadline))
            .unwrap_or_default(),
        stderr: stderr
            .map(|reader| collect(&reader, deadline))
            .unwrap_or_default(),
        timed_out,
        duration: started.elapsed(),
    })
}

fn wait(child: &mut Child, timeout: Duration) -> Result<(Option<i32>, bool)> {
    let deadline = Instant::now() + timeout;
    loop {
        let status = child
            .try_wait()
            .map_err(|error| BenchError::Process(format!("cannot wait for a child: {error}")))?;
        if let Some(status) = status {
            return Ok((status.code(), false));
        }
        if Instant::now() >= deadline {
            kill_tree(child);
            let _ = child.wait();
            return Ok((None, true));
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Kills a child together with its process group (Unix) or its process
/// tree (Windows).
pub fn kill_tree(child: &mut Child) {
    let pid = child.id().to_string();
    #[cfg(unix)]
    let killer = Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    #[cfg(windows)]
    let killer = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    #[cfg(not(any(unix, windows)))]
    let killer: std::io::Result<std::process::ExitStatus> =
        Err(std::io::ErrorKind::Unsupported.into());
    if !killer.is_ok_and(|status| status.success()) {
        let _ = child.kill();
    }
}

fn spawn_reader(mut source: impl Read + Send + 'static) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = vec![0_u8; READ_CHUNK];
        loop {
            match source.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if sender.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    receiver
}

fn collect(receiver: &Receiver<Vec<u8>>, deadline: Instant) -> Vec<u8> {
    let mut output = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(left) {
            Ok(chunk) => output.extend_from_slice(&chunk),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return output,
        }
    }
}

/// Splits a command line into words: words are separated by white space,
/// and single or double quotes group words.
///
/// ```
/// use nurt_bench::process::split_command;
///
/// assert_eq!(
///     split_command(r#"nurt serve --stdio --name "a b" 'c d'"#).unwrap(),
///     ["nurt", "serve", "--stdio", "--name", "a b", "c d"]
/// );
/// ```
///
/// # Errors
///
/// Returns [`BenchError::Argument`] for an unclosed quote or an empty
/// command.
pub fn split_command(text: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for character in text.chars() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => current.push(character),
            (None, '"' | '\'') => {
                quote = Some(character);
                started = true;
            }
            (None, _) if character.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            (None, _) => {
                current.push(character);
                started = true;
            }
        }
    }
    if quote.is_some() {
        return Err(BenchError::Argument(format!(
            "unclosed quote in command {text:?}"
        )));
    }
    if started {
        words.push(current);
    }
    if words.is_empty() {
        return Err(BenchError::Argument("the command is empty".to_owned()));
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(script: &str, timeout: Duration) -> ProcessSpec {
        ProcessSpec::new(
            "sh",
            vec!["-c".into(), script.into()],
            std::env::temp_dir(),
            timeout,
        )
    }

    #[test]
    fn splits_words_and_quotes() {
        assert_eq!(split_command("a  b\tc").unwrap(), ["a", "b", "c"]);
        assert_eq!(split_command(r#"x "" y"#).unwrap(), ["x", "", "y"]);
        assert_eq!(split_command(r#"a"b c"d"#).unwrap(), ["ab cd"]);
        assert!(split_command("a 'b").is_err());
        assert!(split_command("   ").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn captures_output_and_exit_code() {
        let output = run(&shell(
            "printf out; printf err >&2; exit 3",
            Duration::from_secs(10),
        ))
        .unwrap();
        assert_eq!(output.stdout, b"out");
        assert_eq!(output.stderr, b"err");
        assert_eq!(output.code, Some(3));
        assert!(!output.success());
        assert!(!output.timed_out);
    }

    #[cfg(unix)]
    #[test]
    fn passes_standard_input_and_environment() {
        let mut spec = shell("cat; printf \"$NB_TEST\"", Duration::from_secs(10));
        spec.stdin = Some(b"in-".to_vec());
        spec.env.push(("NB_TEST".into(), "env".into()));
        let output = run(&spec).unwrap();
        assert_eq!(output.stdout, b"in-env");
        assert!(output.success());
    }

    #[cfg(unix)]
    #[test]
    fn kills_a_process_that_exceeds_the_limit() {
        let output = run(&shell("sleep 5 & sleep 5", Duration::from_millis(200))).unwrap();
        assert!(output.timed_out);
        assert_eq!(output.code, None);
        assert!(output.duration < Duration::from_secs(4));
    }

    #[test]
    fn reports_a_program_that_cannot_start() {
        let spec = ProcessSpec::new(
            "nurt-bench-no-such-program",
            Vec::new(),
            std::env::temp_dir(),
            Duration::from_secs(1),
        );
        assert!(matches!(run(&spec), Err(BenchError::Process(_))));
    }
}
