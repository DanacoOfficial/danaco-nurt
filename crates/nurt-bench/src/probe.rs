// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Probes of state outside the file tree, collected after a run.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::error::Result;
use crate::process::{self, ProcessSpec};
use crate::task::{HostOs, Probe};

/// Time limit of one probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Normalizes the standard output of a probe: UTF-8, line feeds only, no
/// trailing white space.
#[must_use]
pub fn normalize_stdout(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace("\r\n", "\n")
        .trim_end()
        .to_owned()
}

/// Collects one probe. A probe that does not apply to the system returns an
/// object with the member `error`, which no expectation matches.
///
/// # Errors
///
/// Returns an error when a probe process cannot be started.
pub fn collect(
    probe: &Probe,
    root: &Path,
    env: &[(String, String)],
    host: HostOs,
) -> Result<Value> {
    match probe.kind.as_str() {
        "command" => command(probe, root, env),
        "process" => match probe.pattern.as_deref() {
            Some(pattern) if !pattern.is_empty() => {
                Ok(json!({"count": count_processes(pattern, root, env, host)?}))
            }
            _ => Ok(json!({"error": "the probe has no pattern"})),
        },
        "registry" if host == HostOs::Windows => registry(probe, root),
        "scheduled_task" if host == HostOs::Windows => scheduled_task(probe, root),
        "registry" | "scheduled_task" => Ok(json!({"error": "the probe applies to Windows only"})),
        other => Ok(json!({"error": format!("unknown probe kind {other}")})),
    }
}

/// Result of a command probe; a probe that was stopped at its time limit has
/// no result, only an error, because its output is incomplete.
fn command_result(output: process::ProcessOutput) -> Value {
    if output.timed_out {
        return json!({
            "error": format!("the probe did not finish within {} seconds", PROBE_TIMEOUT.as_secs())
        });
    }
    json!({"exit": output.code, "stdout": normalize_stdout(&output.stdout)})
}

fn command(probe: &Probe, root: &Path, env: &[(String, String)]) -> Result<Value> {
    let Some((program, args)) = probe.argv.split_first() else {
        return Ok(json!({"error": "the probe has no argv"}));
    };
    let mut spec = ProcessSpec::new(program, args.to_vec(), root.to_path_buf(), PROBE_TIMEOUT);
    spec.env = env.to_vec();
    match process::run(&spec) {
        Ok(output) => Ok(command_result(output)),
        Err(error) => Ok(json!({"error": error.to_string()})),
    }
}

/// Number of processes whose command line contains `pattern`. On Linux only
/// the processes of the run are counted: those that work in the task root or
/// carry the home directory of the run in their environment, so that the
/// processes of other runs and of the host are not counted. The other
/// systems give no working directory or environment, so they count the
/// descendants of this process and the processes whose command line names the
/// task root or the home directory of the run (see [`select_run_rows`]).
fn count_processes(
    pattern: &str,
    root: &Path,
    env: &[(String, String)],
    host: HostOs,
) -> Result<usize> {
    let home = env
        .iter()
        .find(|(name, _)| name == "HOME")
        .map(|(_, value)| value.as_str());
    let lines = match host {
        HostOs::Linux => proc_command_lines(root, home),
        HostOs::Macos => {
            let spec = ProcessSpec::new(
                "ps",
                vec!["-A".into(), "-o".into(), "pid=,ppid=,command=".into()],
                root.to_path_buf(),
                PROBE_TIMEOUT,
            );
            let output = process::run(&spec)?;
            select_run_rows(
                &parse_rows(&normalize_stdout(&output.stdout)),
                std::process::id(),
                root,
                home,
            )
        }
        HostOs::Windows => {
            let script = "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId) $($_.CommandLine)\" }";
            let output = process::run(&pwsh(script, root))?;
            select_run_rows(
                &parse_rows(&normalize_stdout(&output.stdout)),
                std::process::id(),
                root,
                home,
            )
        }
    };
    Ok(lines
        .iter()
        .filter(|line| line.contains(pattern) && !line.contains("Win32_Process"))
        .count())
}

/// One process of a listing: identifier, parent identifier and command line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProcessRow {
    pid: u32,
    ppid: u32,
    command: String,
}

/// Parses lines of the form `<pid> <ppid> <command line>`; other lines are
/// skipped.
fn parse_rows(text: &str) -> Vec<ProcessRow> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.trim_start().splitn(3, char::is_whitespace);
            let pid = parts.next()?.parse().ok()?;
            let rest = parts.next()?;
            let ppid = rest.parse().ok()?;
            let command = parts.next().unwrap_or_default().trim().to_owned();
            Some(ProcessRow { pid, ppid, command })
        })
        .collect()
}

/// Command lines of the processes of the run: the descendants of `own` and the
/// processes whose command line names `root` or `home`. The process `own`, the
/// listing command and the process that queries the processes are left out.
fn select_run_rows(rows: &[ProcessRow], own: u32, root: &Path, home: Option<&str>) -> Vec<String> {
    let root = root.to_string_lossy();
    let descends = |start: &ProcessRow| {
        let mut current = start.ppid;
        for _ in 0..rows.len() {
            if current == own {
                return true;
            }
            match rows.iter().find(|row| row.pid == current) {
                Some(parent) if parent.pid != parent.ppid => current = parent.ppid,
                _ => return false,
            }
        }
        false
    };
    rows.iter()
        .filter(|row| row.pid != own && !row.command.is_empty())
        .filter(|row| !row.command.contains("Win32_Process") && !row.command.starts_with("ps -A"))
        .filter(|row| {
            descends(row)
                || row.command.contains(root.as_ref())
                || home.is_some_and(|home| !home.is_empty() && row.command.contains(home))
        })
        .map(|row| row.command.clone())
        .collect()
}

/// Whether the process whose `/proc` directory is `directory` works in
/// `root` or has `HOME=<home>` in its environment.
fn belongs_to_run(directory: &Path, root: &Path, home: Option<&str>) -> bool {
    if std::fs::read_link(directory.join("cwd")).is_ok_and(|cwd| cwd.starts_with(root)) {
        return true;
    }
    let Some(home) = home else { return false };
    let wanted = format!("HOME={home}");
    std::fs::read(directory.join("environ")).is_ok_and(|raw| {
        raw.split(|byte| *byte == 0)
            .any(|var| var == wanted.as_bytes())
    })
}

fn proc_command_lines(root: &Path, home: Option<&str>) -> Vec<String> {
    let own = std::process::id().to_string();
    let Ok(listing) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    listing
        .filter_map(std::result::Result::ok)
        .filter(|item| {
            let name = item.file_name();
            let name = name.to_string_lossy();
            name.bytes().all(|byte| byte.is_ascii_digit()) && name != own.as_str()
        })
        .filter(|item| belongs_to_run(&item.path(), root, home))
        .filter_map(|item| std::fs::read(item.path().join("cmdline")).ok())
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            String::from_utf8_lossy(&raw)
                .replace('\0', " ")
                .trim_end()
                .to_owned()
        })
        .collect()
}

fn quote_pwsh(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

fn pwsh(script: &str, root: &Path) -> ProcessSpec {
    ProcessSpec::new(
        "pwsh",
        vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            script.into(),
        ],
        root.to_path_buf(),
        PROBE_TIMEOUT,
    )
}

fn pwsh_json(script: &str, root: &Path) -> Result<Value> {
    let output = process::run(&pwsh(script, root))?;
    Ok(serde_json::from_str(&normalize_stdout(&output.stdout))
        .unwrap_or_else(|_| json!({"error": normalize_stdout(&output.stderr)})))
}

fn registry(probe: &Probe, root: &Path) -> Result<Value> {
    let key = quote_pwsh(&format!(
        "Registry::{}",
        probe.key.as_deref().unwrap_or_default()
    ));
    let value = quote_pwsh(probe.value.as_deref().unwrap_or_default());
    let script = format!(
        "$i = Get-ItemProperty -LiteralPath {key} -Name {value} -ErrorAction SilentlyContinue; \
         if ($null -eq $i) {{ '{{\"exists\":false,\"data\":null}}' }} \
         else {{ @{{exists=$true; data=$i.{value}}} | ConvertTo-Json -Compress }}"
    );
    pwsh_json(&script, root)
}

fn scheduled_task(probe: &Probe, root: &Path) -> Result<Value> {
    let path = quote_pwsh(probe.path.as_deref().unwrap_or("\\"));
    let name = quote_pwsh(probe.name.as_deref().unwrap_or_default());
    let script = format!(
        "$t = Get-ScheduledTask -TaskPath {path} -TaskName {name} -ErrorAction SilentlyContinue; \
         if ($null -eq $t) {{ '{{\"exists\":false}}' }} \
         else {{ $a = $t.Actions | Select-Object -First 1; \
         @{{exists=$true; enabled=($t.State -ne 'Disabled'); execute=$a.Execute; arguments=$a.Arguments}} \
         | ConvertTo-Json -Compress }}"
    );
    pwsh_json(&script, root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(kind: &str) -> Probe {
        Probe {
            id: "p".into(),
            kind: kind.into(),
            argv: Vec::new(),
            pattern: None,
            key: None,
            value: None,
            path: None,
            name: None,
        }
    }

    #[test]
    fn normalizes_output() {
        assert_eq!(normalize_stdout(b"a\r\nb  \n\n"), "a\nb");
        assert_eq!(quote_pwsh("it's"), "'it''s'");
    }

    #[cfg(unix)]
    #[test]
    fn runs_command_probes_in_the_task_root() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("f.txt"), "content\r\n").unwrap();
        let mut cat = probe("command");
        cat.argv = vec!["cat".into(), "f.txt".into()];
        let result = collect(&cat, root.path(), &[], HostOs::Linux).unwrap();
        assert_eq!(result, json!({"exit": 0, "stdout": "content"}));
        let empty = collect(&probe("command"), root.path(), &[], HostOs::Linux).unwrap();
        assert!(empty.get("error").is_some());
        let mut missing = probe("command");
        missing.argv = vec!["nurt-bench-no-such-program".into()];
        let result = collect(&missing, root.path(), &[], HostOs::Linux).unwrap();
        assert!(result.get("error").is_some());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn counts_processes_by_command_line() {
        let root = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let home = tempfile::tempdir().unwrap();
        let home_text = home.path().display().to_string();
        let sleep = |directory: &Path, home: Option<&str>| {
            let mut command = std::process::Command::new("sleep");
            command.arg("7.25").current_dir(directory);
            if let Some(home) = home {
                command.env("HOME", home);
            }
            command.spawn().unwrap()
        };
        let mut own = sleep(root.path(), None);
        let mut foreign = sleep(elsewhere.path(), None);
        let mut by_home = sleep(elsewhere.path(), Some(&home_text));
        let mut by_line = probe("process");
        by_line.pattern = Some("sleep 7.25".into());
        let env = [("HOME".to_owned(), home_text.clone())];
        let counted = collect(&by_line, root.path(), &env, HostOs::Linux).unwrap();
        let without_home = collect(&by_line, root.path(), &[], HostOs::Linux).unwrap();
        for child in [&mut own, &mut foreign, &mut by_home] {
            child.kill().unwrap();
            child.wait().unwrap();
        }
        // The process in the root and the one with the home of the run; the
        // process of another directory and another home is not counted.
        assert_eq!(counted["count"], 2);
        assert_eq!(without_home["count"], 1);
    }

    #[test]
    fn rows_of_a_process_listing_are_parsed_and_filtered_to_the_run() {
        let listing = "  100 1 /sbin/launchd\n 200 100 bench --run\n 300 200 sleep 99\n\
                       400 300 sleep 98\n 500 1 sleep 97\n 600 1 sleep /tmp/run-1/x\n\
                       700 1 sleep /home/run-1/y\n 800 200 ps -A -o pid=,ppid=,command=\n\
                       not a row\n 900 1\n";
        let rows = parse_rows(listing);
        assert_eq!(rows.len(), 9);
        assert_eq!(rows[2].command, "sleep 99");
        let own = 200;
        let selected = select_run_rows(&rows, own, Path::new("/tmp/run-1"), Some("/home/run-1"));
        assert_eq!(
            selected,
            vec![
                "sleep 99",
                "sleep 98",
                "sleep /tmp/run-1/x",
                "sleep /home/run-1/y"
            ]
        );
        let without_home = select_run_rows(&rows, own, Path::new("/tmp/run-1"), None);
        assert_eq!(without_home.len(), 3);
    }

    #[test]
    fn a_cyclic_listing_does_not_hang_the_selection() {
        let rows = parse_rows("10 20 a\n20 10 b\n30 30 c\n");
        assert!(select_run_rows(&rows, 1, Path::new("/x"), None).is_empty());
    }

    #[test]
    fn a_process_probe_without_a_pattern_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        for pattern in [None, Some(String::new())] {
            let mut by_line = probe("process");
            by_line.pattern = pattern;
            let result = collect(&by_line, root.path(), &[], HostOs::Linux).unwrap();
            assert!(result.get("error").is_some());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_command_probe_that_times_out_is_an_error() {
        let result = command_result(process::ProcessOutput {
            code: None,
            stdout: b"partial".to_vec(),
            stderr: Vec::new(),
            timed_out: true,
            duration: Duration::from_secs(30),
        });
        assert!(result["error"].as_str().unwrap().contains("did not finish"));
        assert!(result.get("stdout").is_none());
    }

    #[test]
    fn windows_probes_do_not_apply_elsewhere() {
        let root = tempfile::tempdir().unwrap();
        for kind in ["registry", "scheduled_task", "unknown"] {
            let result = collect(&probe(kind), root.path(), &[], HostOs::Linux).unwrap();
            assert!(result.get("error").is_some(), "{kind}");
        }
    }
}
