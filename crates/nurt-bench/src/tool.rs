// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-bench::tool
// Responsibility:   Tool executors: shell processes for bash and PowerShell, and `nurt.
// Dependencies:     nurt_contracts, serde_json
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Tool executors: shell processes for bash and PowerShell, and `nurt
//! serve` for Danaco Nurt.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nurt_contracts::envelope::Request;
use serde_json::{Value, json};

use crate::error::{BenchError, Result};
use crate::process::{self, ProcessSpec};
use crate::rpc::RpcClient;
use crate::runlog::NurtCall;
use crate::task::HostOs;

/// Version of the request envelope sent to `run` and `check`.
const ENVELOPE_VERSION: &str = "1.0";

/// Report statuses that mean the program failed.
const FAILED_STATUSES: [&str; 2] = ["failed", "rolled_back"];

/// Shell of a condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    /// bash.
    Bash,
    /// PowerShell 7.
    Pwsh,
}

impl Shell {
    /// Shell that runs the setup scripts on a system.
    #[must_use]
    pub const fn of_host(host: HostOs) -> Self {
        match host {
            HostOs::Windows => Self::Pwsh,
            HostOs::Linux | HostOs::Macos => Self::Bash,
        }
    }

    /// Process specification that runs a command with this shell, without
    /// profiles and without interaction.
    #[must_use]
    pub fn spec(
        self,
        command: &str,
        cwd: &Path,
        env: &[(String, String)],
        timeout: Duration,
    ) -> ProcessSpec {
        let (program, flags): (&str, &[&str]) = match self {
            Self::Bash => ("bash", &["--noprofile", "--norc", "-c"]),
            Self::Pwsh => ("pwsh", &["-NoProfile", "-NonInteractive", "-Command"]),
        };
        let mut args: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
        args.push(command.to_owned());
        let mut spec = ProcessSpec::new(program, args, cwd.to_path_buf(), timeout);
        spec.env = env.to_vec();
        spec
    }
}

/// Environment of every process of a run: UTF-8 locale, UTC and a home
/// directory outside the task root.
#[must_use]
pub fn run_environment(home: &Path) -> Vec<(String, String)> {
    let home = home.display().to_string();
    vec![
        ("LC_ALL".to_owned(), "C.UTF-8".to_owned()),
        ("TZ".to_owned(), "UTC".to_owned()),
        ("HOME".to_owned(), home.clone()),
        ("USERPROFILE".to_owned(), home),
    ]
}

/// Result of one tool call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ToolOutcome {
    /// Text returned to the model.
    pub output: String,
    /// Whether the call failed.
    pub is_error: bool,
    /// Exit code of a shell call.
    pub exit: Option<i32>,
    /// Whether the call exceeded its time limit.
    pub timed_out: bool,
    /// Report fields of a Danaco Nurt call.
    pub nurt: Option<NurtCall>,
    /// Group descriptions loaded by this call: name and text.
    pub loaded_groups: Vec<(String, String)>,
}

impl ToolOutcome {
    fn invalid(message: String) -> Self {
        Self {
            output: message,
            is_error: true,
            ..Self::default()
        }
    }
}

/// Executor of the tool of one run.
pub trait ToolExecutor {
    /// Executes one call with the arguments given by the model.
    ///
    /// # Errors
    ///
    /// Returns an error only when the executor itself fails (the server
    /// process ended); failures of the command are part of the outcome.
    fn call(&mut self, input: &Value, timeout: Duration) -> Result<ToolOutcome>;

    /// Ends the executor and releases its processes.
    fn finish(self: Box<Self>);
}

/// Executor of shell commands: every call is a new process in the task
/// root.
pub struct ShellTool {
    shell: Shell,
    root: PathBuf,
    env: Vec<(String, String)>,
    argument: String,
}

impl ShellTool {
    /// Creates a shell executor.
    #[must_use]
    pub fn new(shell: Shell, root: PathBuf, env: Vec<(String, String)>, argument: String) -> Self {
        Self {
            shell,
            root,
            env,
            argument,
        }
    }
}

/// Text of a finished shell command as shown to the model: standard output,
/// then standard error after a marker line, then the exit code when it is
/// not 0 or the time limit when it was exceeded.
#[must_use]
pub fn shell_output(output: &process::ProcessOutput, timeout: Duration) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str("[stderr]\n");
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if output.timed_out {
        text.push_str(&format!(
            "[killed after the time limit of {} s]",
            timeout.as_secs()
        ));
    } else if let Some(code) = output.code.filter(|code| *code != 0) {
        text.push_str(&format!("[exit code {code}]"));
    } else if output.code.is_none() {
        text.push_str("[terminated by a signal]");
    }
    text
}

impl ToolExecutor for ShellTool {
    fn call(&mut self, input: &Value, timeout: Duration) -> Result<ToolOutcome> {
        let Some(command) = input.get(&self.argument).and_then(Value::as_str) else {
            return Ok(ToolOutcome::invalid(format!(
                "the argument {:?} with the command text is required",
                self.argument
            )));
        };
        let output = process::run(&self.shell.spec(command, &self.root, &self.env, timeout))?;
        Ok(ToolOutcome {
            output: shell_output(&output, timeout),
            is_error: !output.success(),
            exit: output.code,
            timed_out: output.timed_out,
            nurt: None,
            loaded_groups: Vec::new(),
        })
    }

    fn finish(self: Box<Self>) {}
}

/// Executor of Danaco Nurt programs through `nurt serve`.
///
/// Every request gets `options.base` set to the task root. With groups on
/// demand, a program is first checked; the description of every group it
/// uses that was not shown before is returned with the result.
pub struct NurtTool {
    client: RpcClient,
    root: String,
    groups_on_demand: bool,
    loaded: BTreeSet<String>,
}

impl NurtTool {
    /// Creates an executor over a connected server.
    #[must_use]
    pub fn new(client: RpcClient, root: &Path, groups_on_demand: bool) -> Self {
        Self {
            client,
            root: root.display().to_string().replace('\\', "/"),
            groups_on_demand,
            loaded: BTreeSet::new(),
        }
    }

    fn envelope(&self, input: &Value, timeout: Duration) -> std::result::Result<Request, String> {
        let Value::Object(fields) = input else {
            return Err("the arguments must be an object with the member \"program\"".to_owned());
        };
        let mut envelope = Value::Object(fields.clone());
        envelope["nurt"] = json!(ENVELOPE_VERSION);
        let options = envelope
            .as_object_mut()
            .map(|object| object.entry("options").or_insert_with(|| json!({})));
        if let Some(Value::Object(options)) = options {
            options.insert("base".to_owned(), json!(self.root));
            options.insert(
                "timeout".to_owned(),
                json!(format!("{}s", timeout.as_secs().max(1))),
            );
        }
        serde_json::from_value(envelope).map_err(|error| format!("invalid arguments: {error}"))
    }

    fn load_groups(&mut self, request: &Request) -> Result<Vec<(String, String)>> {
        let check = self
            .client
            .request("check", request, crate::rpc::CALL_TIMEOUT)?;
        let used: Vec<String> = check["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|group| group.as_str().or_else(|| group["name"].as_str()))
            .filter(|group| *group != "core")
            .map(str::to_owned)
            .collect();
        let mut loaded = Vec::new();
        for group in used {
            if self.loaded.insert(group.clone()) {
                let text = self.client.describe(Some(&group))?;
                loaded.push((group, text));
            }
        }
        Ok(loaded)
    }
}

/// Report fields of a `nurt-response/1` document.
#[must_use]
pub fn nurt_call(response: &Value) -> NurtCall {
    let report = &response["report"];
    let corrections = report["corrections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|correction| correction["rule"].as_str().map(str::to_owned))
        .collect();
    let mut changes: Vec<String> = Vec::new();
    for change in report["changes"].as_array().into_iter().flatten() {
        for key in ["path", "from"] {
            if let Some(path) = change[key].as_str() {
                changes.push(path.to_owned());
            }
        }
    }
    NurtCall {
        status: report["status"].as_str().map(str::to_owned),
        error_code: report["error"]["code"].as_str().map(str::to_owned),
        corrections,
        changes,
    }
}

impl ToolExecutor for NurtTool {
    fn call(&mut self, input: &Value, timeout: Duration) -> Result<ToolOutcome> {
        let request = match self.envelope(input, timeout) {
            Ok(request) => request,
            Err(message) => return Ok(ToolOutcome::invalid(message)),
        };
        let loaded_groups = if self.groups_on_demand {
            self.load_groups(&request)?
        } else {
            Vec::new()
        };
        let response = match self.client.request("run", &request, timeout) {
            Ok(response) => response,
            Err(BenchError::Serve(message)) if message.starts_with("no answer") => {
                return Ok(ToolOutcome {
                    output: format!("[killed after the time limit of {} s]", timeout.as_secs()),
                    is_error: true,
                    timed_out: true,
                    loaded_groups,
                    ..ToolOutcome::default()
                });
            }
            Err(error) => return Err(error),
        };
        let call = nurt_call(&response);
        let output = response["rendered"]["text"]
            .as_str()
            .map_or_else(|| response["report"].to_string(), str::to_owned);
        let is_error = call.error_code.is_some()
            || call
                .status
                .as_deref()
                .is_some_and(|status| FAILED_STATUSES.contains(&status));
        Ok(ToolOutcome {
            output,
            is_error,
            exit: None,
            timed_out: false,
            nurt: Some(call),
            loaded_groups,
        })
    }

    fn finish(self: Box<Self>) {
        self.client.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::rpc::testing::connect;

    #[cfg(unix)]
    #[test]
    fn shell_tool_runs_commands_in_the_task_root() {
        let root = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let mut tool: Box<dyn ToolExecutor> = Box::new(ShellTool::new(
            Shell::Bash,
            root.path().to_path_buf(),
            run_environment(home.path()),
            "command".into(),
        ));
        let outcome = tool
            .call(
                &json!({"command": "printf x > a.txt; printf \"$TZ\"; echo oops >&2; exit 2"}),
                Duration::from_secs(10),
            )
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "x"
        );
        assert_eq!(outcome.output, "UTC\n[stderr]\noops\n[exit code 2]");
        assert!(outcome.is_error);
        assert_eq!(outcome.exit, Some(2));
        let missing = tool
            .call(&json!({"cmd": "ls"}), Duration::from_secs(1))
            .unwrap();
        assert!(missing.is_error);
        let slow = tool
            .call(&json!({"command": "sleep 3"}), Duration::from_millis(100))
            .unwrap();
        assert!(slow.timed_out);
        assert!(slow.output.contains("time limit"));
        tool.finish();
    }

    #[test]
    fn formats_shell_output() {
        let output = process::ProcessOutput {
            code: Some(0),
            stdout: b"a\n".to_vec(),
            stderr: Vec::new(),
            timed_out: false,
            duration: Duration::ZERO,
        };
        assert_eq!(shell_output(&output, Duration::from_secs(1)), "a\n");
        let signal = process::ProcessOutput {
            code: None,
            stdout: Vec::new(),
            ..output
        };
        assert_eq!(
            shell_output(&signal, Duration::from_secs(1)),
            "[terminated by a signal]"
        );
    }

    #[test]
    fn shells_use_profiles_free_invocations() {
        let spec = Shell::Pwsh.spec("Get-Date", Path::new("/tmp"), &[], Duration::from_secs(1));
        assert_eq!(spec.program, "pwsh");
        assert_eq!(
            spec.args,
            ["-NoProfile", "-NonInteractive", "-Command", "Get-Date"]
        );
        assert_eq!(Shell::of_host(HostOs::Macos), Shell::Bash);
        assert_eq!(Shell::of_host(HostOs::Windows), Shell::Pwsh);
    }

    fn server() -> RpcClient {
        connect(Arc::new(|method: &str, params: &Value| match method {
            "check" => Ok(json!({"groups": ["core", {"name": "git"}]})),
            "describe" => {
                Ok(json!({"text": format!("about {}", params["name"].as_str().unwrap())}))
            }
            "run" if params["program"] == "hang" => {
                std::thread::sleep(Duration::from_millis(1500));
                Ok(json!(null))
            }
            "run" if params["program"] == "bad" => Ok(json!({
                "nurt": "1.0",
                "report": {"nurt": "1.0", "status": "failed", "kind": "remove",
                           "error": {"code": "E_AMBIGUOUS", "message": "two candidates"}}
            })),
            "run" => Ok(json!({
                "nurt": "1.0",
                "report": {
                    "nurt": "1.0", "status": "ok", "kind": "move",
                    "corrections": [{"rule": "B03"}],
                    "changes": [{"action": "rename", "path": "b.txt", "from": "a.txt"}],
                    "base": params["options"]["base"]
                },
                "rendered": {"format": "text", "text": format!("moved in {}", params["options"]["base"].as_str().unwrap())}
            })),
            _ => Err((-32601, "unknown".to_owned())),
        }))
    }

    #[test]
    fn nurt_tool_sets_base_and_reads_the_report() {
        let root = Path::new("/home/user/task");
        let mut tool: Box<dyn ToolExecutor> = Box::new(NurtTool::new(server(), root, false));
        let outcome = tool
            .call(
                &json!({"program": "mv a.txt b.txt"}),
                Duration::from_secs(5),
            )
            .unwrap();
        assert_eq!(outcome.output, "moved in /home/user/task");
        assert!(!outcome.is_error);
        let call = outcome.nurt.unwrap();
        assert_eq!(call.status.as_deref(), Some("ok"));
        assert_eq!(call.corrections, ["B03"]);
        assert_eq!(call.changes, ["b.txt", "a.txt"]);
        assert!(outcome.loaded_groups.is_empty());
        let failed = tool
            .call(&json!({"program": "bad"}), Duration::from_secs(5))
            .unwrap();
        assert!(failed.is_error);
        assert_eq!(
            failed.nurt.unwrap().error_code.as_deref(),
            Some("E_AMBIGUOUS")
        );
        assert!(failed.output.contains("E_AMBIGUOUS"));
        tool.finish();
    }

    #[test]
    fn nurt_tool_rejects_invalid_arguments() {
        let mut tool = NurtTool::new(server(), Path::new("/r"), false);
        let outcome = tool.call(&json!("find"), Duration::from_secs(5)).unwrap();
        assert!(outcome.is_error && outcome.output.contains("object"));
        let unknown = tool
            .call(
                &json!({"program": "x", "colour": 1}),
                Duration::from_secs(5),
            )
            .unwrap();
        assert!(unknown.is_error && unknown.output.contains("invalid arguments"));
    }

    #[test]
    fn nurt_tool_loads_each_group_once() {
        let mut tool = NurtTool::new(server(), Path::new("/r"), true);
        let first = tool
            .call(&json!({"program": "git.status"}), Duration::from_secs(5))
            .unwrap();
        assert_eq!(
            first.loaded_groups,
            [("git".to_owned(), "about git".to_owned())]
        );
        let second = tool
            .call(&json!({"program": "git.log"}), Duration::from_secs(5))
            .unwrap();
        assert!(second.loaded_groups.is_empty());
    }

    #[test]
    fn nurt_tool_reports_a_run_beyond_the_time_limit() {
        let mut tool = NurtTool::new(server(), Path::new("/r"), false);
        let outcome = tool
            .call(&json!({"program": "hang"}), Duration::from_secs(1))
            .unwrap();
        assert!(outcome.timed_out && outcome.is_error);
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
