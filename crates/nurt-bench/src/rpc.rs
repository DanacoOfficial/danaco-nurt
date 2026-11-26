// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Client of `nurt serve`: JSON-RPC 2.0 `nurt-rpc/1`, one message per line.
//!
//! The runner, the variant generator and the pair generator talk to Danaco
//! Nurt only through this protocol, so that any implementation that speaks
//! it, including the fake server of the conformance suite, can be measured.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use nurt_contracts::envelope::Request;
use serde_json::{Value, json};

use crate::error::{BenchError, Result};
use crate::process::{self, ProcessSpec};
use crate::tool::run_environment;

/// Protocol identifier sent in `initialize`.
pub const PROTOCOL: &str = "nurt-rpc/1";

/// Time limit of calls other than `run`.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Time to wait for the server to exit after `shutdown`.
const EXIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Interval between checks whether the server has exited.
const EXIT_POLL: Duration = Duration::from_millis(10);

/// Connection to a server.
pub struct RpcClient {
    writer: Option<Box<dyn Write + Send>>,
    lines: Receiver<String>,
    child: Option<Child>,
    next_id: u64,
}

impl RpcClient {
    /// Creates a client over a pair of streams; a thread reads the lines of
    /// `reader`.
    #[must_use]
    pub fn from_streams(writer: Box<dyn Write + Send>, reader: Box<dyn BufRead + Send>) -> Self {
        let (sender, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in reader.lines() {
                let Ok(line) = line else {
                    break;
                };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            writer: Some(writer),
            lines,
            child: None,
            next_id: 0,
        }
    }

    /// Starts a server process. Standard error of the server is discarded.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Serve`] when the process cannot be started.
    pub fn spawn(spec: &ProcessSpec) -> Result<Self> {
        let mut child = spec
            .command()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                BenchError::Serve(format!("cannot start {}: {error}", spec.program))
            })?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(BenchError::Serve(
                "the server has no standard streams".to_owned(),
            ));
        };
        let mut client = Self::from_streams(Box::new(stdin), Box::new(BufReader::new(stdout)));
        client.child = Some(child);
        Ok(client)
    }

    /// Calls a method and returns its result.
    ///
    /// Notifications of the server are skipped.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Serve`] when the message cannot be sent, the
    /// server closes the connection, answers with a JSON-RPC error or does
    /// not answer within `timeout`.
    pub fn call(&mut self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| BenchError::Serve("the connection is closed".to_owned()))?;
        writeln!(writer, "{message}")
            .and_then(|()| writer.flush())
            .map_err(|error| BenchError::Serve(format!("cannot send {method}: {error}")))?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match self.lines.recv_timeout(left) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    self.cancel(id);
                    return Err(BenchError::Serve(format!(
                        "no answer to {method} within {}s",
                        timeout.as_secs()
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(BenchError::Serve(format!(
                        "the server closed the connection during {method}"
                    )));
                }
            };
            let response: Value = serde_json::from_str(&line)
                .map_err(|error| BenchError::Serve(format!("invalid message: {error}")))?;
            if response.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = response.get("error") {
                return Err(BenchError::Serve(format!(
                    "{method} failed with code {}: {}",
                    error["code"],
                    error["message"].as_str().unwrap_or_default()
                )));
            }
            return Ok(response.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    /// Asks the server to stop the request `id` that exceeded its time limit,
    /// so that the program does not keep changing files after the call has
    /// been reported as timed out. The answer to `cancel` and the late
    /// answer to the request carry other identifiers than the next call and
    /// are skipped by it.
    fn cancel(&mut self, id: u64) {
        self.next_id += 1;
        let message = json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "cancel",
            "params": {"request_id": id},
        });
        if let Some(writer) = self.writer.as_mut() {
            let _ = writeln!(writer, "{message}").and_then(|()| writer.flush());
        }
    }

    /// Negotiates the protocol.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Serve`] when the call fails or the server speaks
    /// another protocol.
    pub fn initialize(&mut self) -> Result<Value> {
        let params = json!({
            "protocol": PROTOCOL,
            "client": {"name": "nurt-bench", "version": env!("CARGO_PKG_VERSION")},
            "capabilities": {"events": false},
        });
        let result = self.call("initialize", params, CALL_TIMEOUT)?;
        match result.get("protocol").and_then(Value::as_str) {
            Some(PROTOCOL) => Ok(result),
            other => Err(BenchError::Serve(format!(
                "the server speaks {other:?}, expected {PROTOCOL}"
            ))),
        }
    }

    /// Returns a description layer: the core (`name` absent) or one group.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Serve`] when the call fails or returns no text.
    pub fn describe(&mut self, group: Option<&str>) -> Result<String> {
        let params = match group {
            Some(name) => json!({"layer": "group", "name": name, "language": "en"}),
            None => json!({"layer": "core", "language": "en"}),
        };
        let result = self.call("describe", params, CALL_TIMEOUT)?;
        result
            .as_str()
            .or_else(|| result.get("text").and_then(Value::as_str))
            .map(str::to_owned)
            .ok_or_else(|| BenchError::Serve("describe returned no text".to_owned()))
    }

    /// Sends a request envelope to `run` or `check`.
    ///
    /// # Errors
    ///
    /// Returns [`BenchError::Serve`] when the call fails.
    pub fn request(&mut self, method: &str, request: &Request, timeout: Duration) -> Result<Value> {
        let params = serde_json::to_value(request)
            .map_err(|error| BenchError::Serve(format!("cannot encode the request: {error}")))?;
        self.call(method, params, timeout)
    }

    /// Ends the session: `shutdown`, end of input, and the end of the server
    /// process, which is killed when it does not exit in time.
    pub fn shutdown(mut self) {
        let _ = self.call("shutdown", json!({}), CALL_TIMEOUT);
        self.close();
    }

    fn close(&mut self) {
        self.writer = None;
        if let Some(mut child) = self.child.take() {
            let deadline = Instant::now() + EXIT_TIMEOUT;
            while Instant::now() < deadline {
                if child.try_wait().is_ok_and(|status| status.is_some()) {
                    return;
                }
                thread::sleep(EXIT_POLL);
            }
            process::kill_tree(&mut child);
            let _ = child.wait();
        }
    }
}

impl Drop for RpcClient {
    fn drop(&mut self) {
        self.close();
    }
}

/// Starts a server for a run. The directory `scratch` receives the state
/// and the configuration of the instance.
pub trait ServeLauncher: Send + Sync {
    /// Starts a server and negotiates the protocol.
    ///
    /// # Errors
    ///
    /// Returns an error when the server cannot be started or initialized.
    fn launch(&self, scratch: &Path) -> Result<RpcClient>;
}

/// Launcher of a server command such as `nurt serve --stdio`.
///
/// The instance gets `NURT_STATE_DIR=<scratch>/state` and
/// `NURT_CONFIG=<scratch>/config/config.toml` (an empty configuration), so
/// that runs never touch the configuration of the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLauncher {
    argv: Vec<String>,
}

impl CommandLauncher {
    /// Creates a launcher from a command line.
    ///
    /// # Errors
    ///
    /// Returns an error when the command line is empty or malformed.
    pub fn new(command: &str) -> Result<Self> {
        Ok(Self {
            argv: process::split_command(command)?,
        })
    }
}

impl ServeLauncher for CommandLauncher {
    fn launch(&self, scratch: &Path) -> Result<RpcClient> {
        let state = scratch.join("state");
        let config_dir = scratch.join("config");
        let home = scratch.join("home");
        for directory in [&state, &config_dir, &home] {
            std::fs::create_dir_all(directory).map_err(|error| BenchError::io(directory, error))?;
        }
        let config = config_dir.join("config.toml");
        std::fs::write(&config, "").map_err(|error| BenchError::io(&config, error))?;
        let mut spec = ProcessSpec::new(
            &self.argv[0],
            self.argv[1..].to_vec(),
            PathBuf::from(scratch),
            CALL_TIMEOUT,
        );
        // The server gets the locale, the time zone and a home directory of
        // the same kind as the shell conditions, so that the conditions
        // differ only in the tool.
        spec.env = run_environment(&home);
        spec.env.extend([
            ("NURT_STATE_DIR".to_owned(), state.display().to_string()),
            ("NURT_CONFIG".to_owned(), config.display().to_string()),
        ]);
        let mut client = RpcClient::spawn(&spec)?;
        client.initialize()?;
        Ok(client)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! In-process server for tests, connected by pipes.

    use std::io::{BufRead, BufReader, Write};
    use std::path::Path;
    use std::thread;

    use serde_json::{Value, json};

    use super::{RpcClient, ServeLauncher};
    use crate::error::Result;

    /// Handler of one method call: the result, or an error code and message.
    pub(crate) type Handler =
        dyn Fn(&str, &Value) -> std::result::Result<Value, (i64, String)> + Send + Sync;

    /// Connects a client to a server thread that answers with `handler`.
    /// `initialize` and `shutdown` are answered by the server itself.
    pub(crate) fn connect(handler: std::sync::Arc<Handler>) -> RpcClient {
        let (client_reader, server_writer) = std::io::pipe().unwrap();
        let (server_reader, client_writer) = std::io::pipe().unwrap();
        thread::spawn(move || {
            let mut writer = server_writer;
            for line in BufReader::new(server_reader).lines() {
                let Ok(line) = line else { break };
                let message: Value = serde_json::from_str(&line).unwrap();
                let method = message["method"].as_str().unwrap_or_default().to_owned();
                let id = message["id"].clone();
                let answer = match method.as_str() {
                    "initialize" => Ok(json!({"protocol": "nurt-rpc/1", "version": "0.0.0"})),
                    "shutdown" => Ok(json!({})),
                    _ => handler(&method, &message["params"]),
                };
                let reply = match answer {
                    Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                    Err((code, text)) => {
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": text}})
                    }
                };
                writeln!(
                    writer,
                    "{{\"jsonrpc\":\"2.0\",\"method\":\"nurt/event\",\"params\":{{}}}}"
                )
                .unwrap();
                writeln!(writer, "{reply}").unwrap();
                if method == "shutdown" {
                    break;
                }
            }
        });
        RpcClient::from_streams(
            Box::new(client_writer),
            Box::new(BufReader::new(client_reader)),
        )
    }

    /// Launcher that connects every run to an in-process server.
    pub(crate) struct FakeLauncher(pub(crate) std::sync::Arc<Handler>);

    impl ServeLauncher for FakeLauncher {
        fn launch(&self, _scratch: &Path) -> Result<RpcClient> {
            let mut client = connect(std::sync::Arc::clone(&self.0));
            client.initialize()?;
            Ok(client)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::testing::{FakeLauncher, connect};
    use super::*;

    fn handler() -> Arc<testing::Handler> {
        Arc::new(|method: &str, params: &Value| match method {
            "describe" if params["layer"] == "core" => Ok(json!({"text": "core description"})),
            "describe" => Ok(json!(format!("group {}", params["name"].as_str().unwrap()))),
            "check" => Ok(json!({"program": params["program"]})),
            "slow" => {
                thread::sleep(Duration::from_millis(300));
                Ok(json!(null))
            }
            _ => Err((-32601, "method not found".to_owned())),
        })
    }

    #[test]
    fn calls_methods_and_skips_notifications() {
        let mut client = FakeLauncher(handler()).launch(Path::new(".")).unwrap();
        assert_eq!(client.describe(None).unwrap(), "core description");
        assert_eq!(client.describe(Some("git")).unwrap(), "group git");
        let request = Request::program("find \".\"");
        let result = client.request("check", &request, CALL_TIMEOUT).unwrap();
        assert_eq!(result["program"], "find \".\"");
        client.shutdown();
    }

    #[test]
    fn reports_errors_and_timeouts() {
        let mut client = connect(handler());
        let error = client.call("unknown", json!({}), CALL_TIMEOUT).unwrap_err();
        assert!(error.to_string().contains("-32601"));
        let error = client
            .call("slow", json!({}), Duration::from_millis(50))
            .unwrap_err();
        assert!(error.to_string().contains("no answer"));
    }

    #[test]
    fn a_request_that_exceeds_its_limit_is_cancelled() {
        let cancelled = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = Arc::clone(&cancelled);
        let mut client = connect(Arc::new(move |method: &str, params: &Value| match method {
            "slow" => {
                thread::sleep(Duration::from_millis(200));
                Ok(json!("late"))
            }
            "cancel" => {
                seen.lock().unwrap().push(params["request_id"].clone());
                Ok(json!({"cancelled": true}))
            }
            "other" => Ok(json!("next")),
            _ => Err((-32601, "method not found".to_owned())),
        }));
        client.initialize().unwrap();
        let error = client
            .call("slow", json!({}), Duration::from_millis(20))
            .unwrap_err();
        assert!(error.to_string().contains("no answer"));
        // The next answer arrives after the server handled the cancellation
        // and skips the late answer of the slow request.
        let next = client.call("other", json!({}), CALL_TIMEOUT).unwrap();
        assert_eq!(next, json!("next"));
        assert_eq!(*cancelled.lock().unwrap(), [json!(2)]);
    }

    #[test]
    fn rejects_another_protocol() {
        let (client_reader, mut server_writer) = std::io::pipe().unwrap();
        let (_server_reader, client_writer) = std::io::pipe().unwrap();
        writeln!(
            server_writer,
            r#"{{"jsonrpc":"2.0","id":1,"result":{{"protocol":"other/2"}}}}"#
        )
        .unwrap();
        let mut client = RpcClient::from_streams(
            Box::new(client_writer),
            Box::new(BufReader::new(client_reader)),
        );
        assert!(client.initialize().is_err());
        drop(server_writer);
        let error = client.call("next", json!({}), CALL_TIMEOUT).unwrap_err();
        assert!(error.to_string().contains("closed"));
    }

    #[cfg(unix)]
    #[test]
    fn the_server_runs_in_the_same_locale_and_time_zone_as_the_shells() {
        let scratch = tempfile::tempdir().unwrap();
        let script = r#"read -r line; printf '{"jsonrpc":"2.0","id":1,"result":{"protocol":"nurt-rpc/1","env":"%s %s %s"}}\n' "$LC_ALL" "$TZ" "$HOME"; read -r line; printf '{"jsonrpc":"2.0","id":2,"result":{}}\n'"#;
        std::fs::write(scratch.path().join("server.sh"), script).unwrap();
        let mut spec = ProcessSpec::new(
            "sh",
            vec!["server.sh".to_owned()],
            scratch.path().to_path_buf(),
            CALL_TIMEOUT,
        );
        spec.env = run_environment(&scratch.path().join("home"));
        let mut client = RpcClient::spawn(&spec).unwrap();
        let result = client.initialize().unwrap();
        let home = scratch.path().join("home");
        assert_eq!(
            result["env"],
            format!("C.UTF-8 UTC {}", home.display()).as_str()
        );
        client.shutdown();
    }

    #[cfg(unix)]
    #[test]
    fn launches_a_server_process_with_isolated_state() {
        let scratch = tempfile::tempdir().unwrap();
        let script = r#"read -r line; printf '{"jsonrpc":"2.0","id":1,"result":{"protocol":"nurt-rpc/1","state":"%s"}}\n' "$NURT_STATE_DIR"; read -r line; printf '{"jsonrpc":"2.0","id":2,"result":{}}\n'"#;
        std::fs::write(scratch.path().join("server.sh"), script).unwrap();
        let launcher = CommandLauncher::new("sh server.sh").unwrap();
        let client = launcher.launch(scratch.path()).unwrap();
        assert!(scratch.path().join("config/config.toml").is_file());
        assert!(scratch.path().join("home").is_dir());
        client.shutdown();
        assert!(CommandLauncher::new("").is_err());
        let missing = CommandLauncher::new("nurt-bench-no-such-server").unwrap();
        assert!(missing.launch(scratch.path()).is_err());
    }
}
