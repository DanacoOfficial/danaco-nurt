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
        "process" => Ok(
            json!({"count": count_processes(probe.pattern.as_deref().unwrap_or_default(), root, host)?}),
        ),
        "registry" if host == HostOs::Windows => registry(probe, root),
        "scheduled_task" if host == HostOs::Windows => scheduled_task(probe, root),
        "registry" | "scheduled_task" => Ok(json!({"error": "the probe applies to Windows only"})),
        other => Ok(json!({"error": format!("unknown probe kind {other}")})),
    }
}

fn command(probe: &Probe, root: &Path, env: &[(String, String)]) -> Result<Value> {
    let Some((program, args)) = probe.argv.split_first() else {
        return Ok(json!({"error": "the probe has no argv"}));
    };
    let mut spec = ProcessSpec::new(program, args.to_vec(), root.to_path_buf(), PROBE_TIMEOUT);
    spec.env = env.to_vec();
    match process::run(&spec) {
        Ok(output) => Ok(json!({"exit": output.code, "stdout": normalize_stdout(&output.stdout)})),
        Err(error) => Ok(json!({"error": error.to_string()})),
    }
}

fn count_processes(pattern: &str, root: &Path, host: HostOs) -> Result<usize> {
    let lines = match host {
        HostOs::Linux => proc_command_lines(),
        HostOs::Macos => {
            let spec = ProcessSpec::new(
                "ps",
                vec!["-A".into(), "-o".into(), "command=".into()],
                root.to_path_buf(),
                PROBE_TIMEOUT,
            );
            normalize_stdout(&process::run(&spec)?.stdout)
                .lines()
                .map(str::to_owned)
                .collect()
        }
        HostOs::Windows => {
            let script = "Get-CimInstance Win32_Process | ForEach-Object { $_.CommandLine }";
            let output = process::run(&pwsh(script, root))?;
            normalize_stdout(&output.stdout)
                .lines()
                .map(str::to_owned)
                .collect()
        }
    };
    Ok(lines
        .iter()
        .filter(|line| line.contains(pattern) && !line.contains("Win32_Process"))
        .count())
}

fn proc_command_lines() -> Vec<String> {
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
        let mut sleeper = std::process::Command::new("sleep")
            .arg("7.25")
            .spawn()
            .unwrap();
        let mut by_line = probe("process");
        by_line.pattern = Some("sleep 7.25".into());
        let result = collect(&by_line, root.path(), &[], HostOs::Linux).unwrap();
        sleeper.kill().unwrap();
        sleeper.wait().unwrap();
        assert_eq!(result["count"], 1);
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
