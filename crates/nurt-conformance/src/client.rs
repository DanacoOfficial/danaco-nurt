// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//! Connection to a server that speaks `nurt-rpc/1`.

use std::ffi::OsString;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::Instant;

use serde_json::Value;

use crate::Error;
use crate::protocol::{self, method};
use crate::schemas::SchemaSet;

/// Error of a call.
#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    /// The server answered with a JSON-RPC error.
    Rpc {
        /// JSON-RPC error code.
        code: i64,
        /// Message of the error.
        message: String,
        /// Data of the error.
        data: Option<Value>,
    },
    /// The server broke the protocol.
    Protocol(String),
    /// The server process ended or could not be reached.
    Transport(String),
    /// The time limit of the case was reached.
    Timeout,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rpc { code, message, .. } => write!(f, "error {code}: {message}"),
            Self::Protocol(message) => write!(f, "protocol violation: {message}"),
            Self::Transport(message) => write!(f, "server not reachable: {message}"),
            Self::Timeout => f.write_str("time limit reached"),
        }
    }
}

impl std::error::Error for CallError {}

/// Connection that sends calls and returns their results.
pub trait Connection {
    /// Calls `method` with `params` and returns the result.
    ///
    /// # Errors
    ///
    /// Returns the JSON-RPC error of the server or a failure of the
    /// connection.
    fn call(&mut self, method: &str, params: &Value) -> Result<Value, CallError>;

    /// Returns the parameters of the notifications `nurt/event` received
    /// since the previous call of this method, in the order of arrival, and
    /// forgets them. A connection without events returns none.
    fn take_events(&mut self) -> Vec<Value> {
        Vec::new()
    }
}

/// Command line that starts a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    words: Vec<String>,
}

impl CommandLine {
    /// Splits a command line into words.
    ///
    /// Words are separated by white space. Single quotes group text
    /// literally; double quotes group text in which a backslash escapes `"`
    /// and `\`; outside quotes a backslash escapes the next character.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty command or an unterminated quote.
    ///
    /// # Examples
    ///
    /// ```
    /// use nurt_conformance::client::CommandLine;
    ///
    /// let command = CommandLine::parse(r#"nurt serve --config "/home/user/my config.toml""#)?;
    /// assert_eq!(command.words(), ["nurt", "serve", "--config", "/home/user/my config.toml"]);
    /// # Ok::<(), nurt_conformance::Error>(())
    /// ```
    pub fn parse(text: &str) -> Result<Self, Error> {
        let mut words = Vec::new();
        let mut current: Option<String> = None;
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            match c {
                c if c.is_whitespace() => words.extend(current.take()),
                '\'' => {
                    let quoted = take_until(&mut chars, '\'', false)
                        .ok_or_else(|| Error::Invalid("unterminated ' in --impl".to_owned()))?;
                    current.get_or_insert_with(String::new).push_str(&quoted);
                }
                '"' => {
                    let quoted = take_until(&mut chars, '"', true)
                        .ok_or_else(|| Error::Invalid("unterminated \" in --impl".to_owned()))?;
                    current.get_or_insert_with(String::new).push_str(&quoted);
                }
                '\\' => {
                    let escaped = chars.next().unwrap_or('\\');
                    current.get_or_insert_with(String::new).push(escaped);
                }
                c => current.get_or_insert_with(String::new).push(c),
            }
        }
        words.extend(current);
        if words.is_empty() {
            return Err(Error::Invalid(
                "--impl is empty; give the command that starts the server".to_owned(),
            ));
        }
        Ok(Self { words })
    }

    /// Returns the words of the command line.
    #[must_use]
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Returns the path of the executable: the first word when it contains a
    /// path separator, otherwise the first match in `PATH`.
    #[must_use]
    pub fn executable(&self) -> Option<PathBuf> {
        let program = Path::new(self.words.first()?);
        if program.components().count() > 1 {
            return program.is_file().then(|| program.to_path_buf());
        }
        let search = std::env::var_os("PATH")?;
        std::env::split_paths(&search)
            .flat_map(|dir| executable_names(program).map(move |name| dir.join(name)))
            .find(|candidate| candidate.is_file())
    }
}

fn executable_names(program: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    let mut names = vec![program.to_path_buf()];
    if cfg!(windows) && program.extension().is_none() {
        names.push(program.with_extension("exe"));
    }
    names.into_iter()
}

fn take_until(chars: &mut std::str::Chars<'_>, end: char, escapes: bool) -> Option<String> {
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            c if c == end => return Some(out),
            '\\' if escapes => match chars.next() {
                Some(next) if next == end || next == '\\' => out.push(next),
                Some(next) => {
                    out.push('\\');
                    out.push(next);
                }
                None => return None,
            },
            c => out.push(c),
        }
    }
    None
}

/// Connection to a server process started for one case.
#[derive(Debug)]
pub struct ProcessConnection {
    child: Child,
    /// Lines for the writer thread, which owns the standard input of the
    /// server; `None` once the input is closed.
    writer: Option<Sender<String>>,
    /// Result of every write of the writer thread.
    written: Receiver<Result<(), String>>,
    messages: Receiver<Result<Value, String>>,
    next_id: u64,
    deadline: Instant,
    events: Vec<Value>,
}

impl ProcessConnection {
    /// Starts the server with a clean environment: only the variables of
    /// [`INHERITED_VARIABLES`] of the host are passed on, then `env`.
    ///
    /// Calls fail with [`CallError::Timeout`] once `deadline` has passed,
    /// also when the server does not read a request;
    /// [`Self::set_deadline`] replaces it.
    ///
    /// # Errors
    ///
    /// Returns an error when the process cannot be started.
    pub fn spawn(
        command: &CommandLine,
        env: &[(String, OsString)],
        deadline: Instant,
    ) -> Result<Self, CallError> {
        let (program, args) = command
            .words()
            .split_first()
            .ok_or_else(|| CallError::Transport("empty command".to_owned()))?;
        let mut command = Command::new(program);
        command.args(args).env_clear();
        for name in INHERITED_VARIABLES {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command
            .envs(env.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| CallError::Transport(format!("cannot start {program}: {error}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CallError::Transport("no standard output".to_owned()))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| CallError::Transport("no standard input".to_owned()))?;
        let (writer, requests) = channel::<String>();
        let (done, written) = channel();
        thread::spawn(move || {
            for line in requests {
                let sent = stdin
                    .write_all(line.as_bytes())
                    .and_then(|()| stdin.flush())
                    .map_err(|error| error.to_string());
                if done.send(sent).is_err() {
                    return;
                }
            }
        });
        let (sender, messages) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let message = match line {
                    Ok(line) if line.trim().is_empty() => continue,
                    Ok(line) => serde_json::from_str(&line)
                        .map_err(|error| format!("output is not JSON: {error}: {line}")),
                    Err(error) => Err(error.to_string()),
                };
                if sender.send(message).is_err() {
                    return;
                }
            }
        });
        Ok(Self {
            writer: Some(writer),
            written,
            child,
            messages,
            next_id: 1,
            deadline,
            events: Vec::new(),
        })
    }

    /// Sets the time after which calls fail with [`CallError::Timeout`] and
    /// [`Self::shutdown`] stops waiting for the process.
    pub fn set_deadline(&mut self, deadline: Instant) {
        self.deadline = deadline;
    }

    /// Returns the events received so far.
    #[must_use]
    pub fn events(&self) -> &[Value] {
        &self.events
    }

    /// Sends `shutdown`, closes standard input and waits for the process.
    pub fn shutdown(mut self) {
        // The process is killed in `drop` when it does not end by itself.
        let _ = self.call(method::SHUTDOWN, &Value::Object(serde_json::Map::new()));
        self.writer = None;
        let _ = self.wait_for_exit();
    }

    fn wait_for_exit(&mut self) -> Option<std::process::ExitStatus> {
        while Instant::now() < self.deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status);
            }
            thread::sleep(EXIT_POLL);
        }
        None
    }

    fn receive(&mut self, id: u64) -> Result<Value, CallError> {
        loop {
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            let message = match self.messages.recv_timeout(remaining) {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => return Err(CallError::Protocol(error)),
                Err(RecvTimeoutError::Timeout) => return Err(CallError::Timeout),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(CallError::Transport(
                        "the server closed its standard output".to_owned(),
                    ));
                }
            };
            SchemaSet::shared()
                .map_err(|error| CallError::Protocol(error.to_string()))?
                .validate_message(&message)
                .map_err(|errors| {
                    CallError::Protocol(format!(
                        "message does not follow nurt-rpc/1: {}: {message}",
                        errors.join("; ")
                    ))
                })?;
            if message.get("method").and_then(Value::as_str) == Some(method::EVENT) {
                self.events
                    .push(message.get("params").cloned().unwrap_or(Value::Null));
                continue;
            }
            return answer(&message, id);
        }
    }
}

/// Variables of the host that the server under test inherits, besides the
/// variables that the suite sets: the search path, the home and temporary
/// directories and the system directory that Windows needs to start a
/// program.
pub const INHERITED_VARIABLES: [&str; 5] = ["PATH", "HOME", "TMPDIR", "TMP", "SYSTEMROOT"];

/// Interval of checks whether the server process has ended.
const EXIT_POLL: std::time::Duration = std::time::Duration::from_millis(10);

fn answer(message: &Value, id: u64) -> Result<Value, CallError> {
    if message.get("id").and_then(Value::as_u64) != Some(id) {
        return Err(CallError::Protocol(format!(
            "expected the response to request {id}, received {message}"
        )));
    }
    if let Some(error) = message.get("error") {
        return Err(CallError::Rpc {
            code: error
                .get("code")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            message: error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            data: error.get("data").cloned(),
        });
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| CallError::Protocol(format!("response without result: {message}")))
}

impl Connection for ProcessConnection {
    fn call(&mut self, method: &str, params: &Value) -> Result<Value, CallError> {
        let id = self.next_id;
        self.next_id += 1;
        let closed = || CallError::Transport("standard input is closed".to_owned());
        let writer = self.writer.as_ref().ok_or_else(closed)?;
        let mut line = protocol::request(id, method, params).to_string();
        line.push('\n');
        // The result of a write that timed out earlier is not the one of this call.
        while self.written.try_recv().is_ok() {}
        writer.send(line).map_err(|_| closed())?;
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        match self.written.recv_timeout(remaining) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                return Err(CallError::Transport(format!(
                    "cannot write a request: {error}"
                )));
            }
            Err(RecvTimeoutError::Timeout) => return Err(CallError::Timeout),
            Err(RecvTimeoutError::Disconnected) => return Err(closed()),
        }
        self.receive(id)
    }

    fn take_events(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.events)
    }
}

impl Drop for ProcessConnection {
    fn drop(&mut self) {
        // The process may already have ended; both results are irrelevant.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_lines_split_on_white_space_and_quotes() {
        let command = CommandLine::parse(r#"  a 'b c'  "d \"e\" \\ \x" f\ g h''  "#).unwrap();
        assert_eq!(command.words(), ["a", "b c", "d \"e\" \\ \\x", "f g", "h"]);
        assert!(CommandLine::parse("   ").is_err());
        assert!(CommandLine::parse("a 'b").is_err());
        assert!(CommandLine::parse("a \"b").is_err());
        assert!(CommandLine::parse("a \"b\\").is_err());
    }

    #[test]
    fn executables_are_found_by_path_or_in_the_search_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("server");
        std::fs::write(&file, "").unwrap();
        let direct = CommandLine::parse(&format!("'{}' serve", file.display())).unwrap();
        assert_eq!(direct.executable(), Some(file));
        let missing = CommandLine::parse("no-such-program-of-the-suite").unwrap();
        assert_eq!(missing.executable(), None);
    }

    #[test]
    fn answers_are_checked_against_the_request() {
        assert_eq!(answer(&json!({"id": 1, "result": 5}), 1), Ok(json!(5)));
        assert!(matches!(
            answer(&json!({"id": 2, "result": 5}), 1),
            Err(CallError::Protocol(_))
        ));
        assert!(matches!(
            answer(&json!({"id": 1}), 1),
            Err(CallError::Protocol(_))
        ));
        let error = answer(
            &json!({"id": 1, "error": {"code": -32003, "message": "m"}}),
            1,
        );
        assert_eq!(
            error,
            Err(CallError::Rpc {
                code: -32003,
                message: "m".to_owned(),
                data: None
            })
        );
    }

    #[test]
    fn call_errors_are_described() {
        let rpc = CallError::Rpc {
            code: -1,
            message: "m".to_owned(),
            data: None,
        };
        assert_eq!(rpc.to_string(), "error -1: m");
        assert!(
            CallError::Protocol("x".to_owned())
                .to_string()
                .contains("protocol")
        );
        assert!(
            CallError::Transport("x".to_owned())
                .to_string()
                .contains("reachable")
        );
        assert_eq!(CallError::Timeout.to_string(), "time limit reached");
    }

    #[cfg(unix)]
    #[test]
    fn the_server_gets_a_clean_environment() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("server.sh");
        std::fs::write(
            &script,
            "read -r line\nprintf '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"host\":\"%s\",\"suite\":\"%s\"}}\\n' \"${CARGO_MANIFEST_DIR:-unset}\" \"${NURT_TEST_VARIABLE:-unset}\"\n",
        )
        .unwrap();
        let command = CommandLine::parse(&format!("sh {}", script.display())).unwrap();
        let env = [("NURT_TEST_VARIABLE".to_owned(), OsString::from("set"))];
        let deadline = Instant::now() + std::time::Duration::from_secs(20);
        let mut connection = ProcessConnection::spawn(&command, &env, deadline).unwrap();
        let result = connection.call("initialize", &json!({})).unwrap();
        assert_eq!(result, json!({"host": "unset", "suite": "set"}));
    }

    #[cfg(unix)]
    #[test]
    fn a_server_that_does_not_read_its_input_ends_in_a_timeout() {
        let command = CommandLine::parse("sleep 30").unwrap();
        let deadline = Instant::now() + std::time::Duration::from_millis(300);
        let mut connection = ProcessConnection::spawn(&command, &[], deadline).unwrap();
        let params = json!({"text": "x".repeat(4 << 20)});
        assert_eq!(
            connection.call("check", &params),
            Err(CallError::Timeout)
        );
    }

    #[test]
    fn missing_programs_fail_to_start() {
        let command = CommandLine::parse("no-such-program-of-the-suite").unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        assert!(matches!(
            ProcessConnection::spawn(&command, &[], deadline),
            Err(CallError::Transport(_))
        ));
    }
}
