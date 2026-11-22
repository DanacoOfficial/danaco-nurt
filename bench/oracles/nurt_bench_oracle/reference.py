# Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
# Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
# https://github.com/DanacoOfficial | support@danaco-group.pl
"""Execution of reference and defective solutions to test the oracles.

For each task the reference solution of the current shell must pass the
oracle, and every defective solution must be rejected. Two defective
solutions are derived for every task: the empty solution, unless the task
accepts the unchanged state, and the reference solution followed by the
deletion of a file that the task does not permit to change.
"""

from __future__ import annotations

import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from .oracle import check, is_allowed
from .snapshot import Index, capture, materialize, resolve_fs_before
from .tasks import FIXTURES_DIR

PROBE_TIMEOUT = 30
KILL_GRACE = 5
GIT_IDENTITY = "[user]\n\tname = Bench User\n\temail = bench@example.com\n[init]\n\tdefaultBranch = main\n"


@dataclass
class Outcome:
    """Result of one solution of a task."""

    label: str
    expected_pass: bool
    verdict: dict[str, Any] | None
    error: str | None = None

    @property
    def ok(self) -> bool:
        """Returns whether the oracle decided as expected."""
        if self.error or self.verdict is None:
            return False
        return bool(self.verdict["passed"]) == self.expected_pass

    def to_json(self) -> dict[str, Any]:
        """Returns the outcome as a JSON object."""
        data: dict[str, Any] = {"label": self.label, "expected_pass": self.expected_pass, "ok": self.ok}
        if self.error:
            data["error"] = self.error
        if self.verdict is not None:
            data["passed"] = self.verdict["passed"]
            data["failures"] = self.verdict["failures"]
            data["harm"] = self.verdict["harm"]
        return data


@dataclass
class Execution:
    """Observed effect of one script on a materialized task."""

    before: Index
    after: Index
    answer: str
    probes: dict[str, Any]
    timed_out: bool


def shell_command(shell: str, script_path: Path) -> list[str]:
    """Returns the command line that runs a script file in a shell."""
    if shell == "bash":
        return ["bash", str(script_path)]
    return ["pwsh", "-NoLogo", "-NoProfile", "-File", str(script_path)]


def base_environment(home: Path) -> dict[str, str]:
    """Returns the environment of every script run of a task."""
    env = dict(os.environ)
    env.update(
        {
            "LC_ALL": "C.UTF-8",
            "LANG": "C.UTF-8",
            "TZ": "UTC",
            "GIT_CONFIG_GLOBAL": str(home / ".gitconfig"),
            "GIT_CONFIG_NOSYSTEM": "1",
            "GIT_TERMINAL_PROMPT": "0",
        }
    )
    if os.name == "posix":
        env["HOME"] = str(home)
    return env


def run_script(
    shell: str, script: str, root: Path, env: Mapping[str, str], timeout: float, scratch: Path
) -> tuple[int | None, str, bool]:
    """Runs a script with the task root as working directory.

    Standard input stays open and empty, as in a tool call that does not
    close it. Returns the exit code, the standard output and whether the
    time limit was reached.
    """
    suffix = ".sh" if shell == "bash" else ".ps1"
    handle, name = tempfile.mkstemp(suffix=suffix, dir=scratch)
    with os.fdopen(handle, "w", encoding="utf-8", newline="\n") as file:
        file.write(script)
    out_path = Path(name + ".out")
    posix = os.name == "posix"
    with out_path.open("wb") as out:
        process = subprocess.Popen(
            shell_command(shell, Path(name)),
            cwd=root,
            env=dict(env),
            stdin=subprocess.PIPE,
            stdout=out,
            stderr=subprocess.DEVNULL,
            start_new_session=posix,
        )
        timed_out = False
        try:
            code: int | None = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            code = None
            stop_process(process, posix)
        finally:
            if process.stdin:
                process.stdin.close()
    output = out_path.read_bytes().decode("utf-8", errors="replace")
    return code, output, timed_out


def stop_process(process: subprocess.Popen[bytes], posix: bool) -> None:
    """Stops a process and, on POSIX systems, its process group."""
    if posix:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
    else:
        process.kill()
    process.wait(timeout=KILL_GRACE)


def count_processes(match: str) -> int:
    """Counts processes whose command line contains ``match``."""
    if os.name == "posix":
        own = os.getpid()
        count = 0
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit() or int(entry.name) == own:
                continue
            try:
                raw = (entry / "cmdline").read_bytes()
            except OSError:
                continue
            if match in raw.replace(b"\0", b" ").decode("utf-8", errors="replace"):
                count += 1
        return count
    script = (
        "$m = $env:NURT_BENCH_MATCH; @(Get-CimInstance Win32_Process | "
        "Where-Object { $_.CommandLine -and $_.CommandLine.Contains($m) -and $_.ProcessId -ne $PID }).Count"
    )
    env = dict(os.environ, NURT_BENCH_MATCH=match)
    result = subprocess.run(
        ["pwsh", "-NoProfile", "-Command", script], capture_output=True, text=True, env=env,
        timeout=PROBE_TIMEOUT, check=False,
    )
    return int(result.stdout.strip() or "0")


def windows_probe(probe: Mapping[str, Any]) -> dict[str, Any]:
    """Collects a registry or scheduled task probe through PowerShell."""
    if probe["kind"] == "registry":
        script = (
            "$p = 'Registry::' + $env:NURT_BENCH_KEY; $n = $env:NURT_BENCH_VALUE; "
            "$item = Get-ItemProperty -LiteralPath $p -Name $n -ErrorAction SilentlyContinue; "
            "if ($null -eq $item) { @{exists=$false} | ConvertTo-Json -Compress } "
            "else { @{exists=$true; data=$item.$n} | ConvertTo-Json -Compress }"
        )
        env = dict(os.environ, NURT_BENCH_KEY=probe["key"], NURT_BENCH_VALUE=probe["value"])
    else:
        script = (
            "$t = Get-ScheduledTask -TaskPath $env:NURT_BENCH_PATH -TaskName $env:NURT_BENCH_NAME "
            "-ErrorAction SilentlyContinue; if ($null -eq $t) { @{exists=$false} | ConvertTo-Json -Compress } "
            "else { @{exists=$true; enabled=($t.State -ne 'Disabled'); execute=[string]$t.Actions[0].Execute; "
            "arguments=[string]$t.Actions[0].Arguments} | ConvertTo-Json -Compress }"
        )
        env = dict(os.environ, NURT_BENCH_PATH=probe["path"], NURT_BENCH_NAME=probe["name"])
    result = subprocess.run(
        ["pwsh", "-NoProfile", "-Command", script], capture_output=True, text=True, env=env,
        timeout=PROBE_TIMEOUT, check=False,
    )
    return json.loads(result.stdout or '{"exists": false}')


def collect_probes(
    probes: list[Mapping[str, Any]], root: Path, env: Mapping[str, str]
) -> dict[str, Any]:
    """Collects the probe results of a task."""
    results: dict[str, Any] = {}
    for probe in probes:
        kind = probe["kind"]
        if kind == "command":
            done = subprocess.run(
                probe["argv"], cwd=root, env=dict(env), capture_output=True,
                timeout=PROBE_TIMEOUT, check=False,
            )
            results[probe["id"]] = {
                "exit": done.returncode,
                "stdout": done.stdout.decode("utf-8", errors="replace"),
            }
        elif kind == "process":
            results[probe["id"]] = {"count": count_processes(probe["match"])}
        else:
            results[probe["id"]] = windows_probe(probe)
    return results


def execute(
    task: Mapping[str, Any], shell: str, script: str, timeout: float, delete: str | None = None
) -> Execution:
    """Materializes a task, runs a script and observes the result.

    ``delete`` names a file of the task root that is removed after the
    script, independent of the directory in which the script ends and of its
    exit code.

    # Errors
    Raises :class:`RuntimeError` when the setup script fails.
    """
    with tempfile.TemporaryDirectory(prefix="nurt-bench-", ignore_cleanup_errors=True) as tmp:
        base = Path(tmp)
        root, home, scratch = base / "root", base / "home", base / "scratch"
        for directory in (root, home, scratch):
            directory.mkdir()
        (home / ".gitconfig").write_text(GIT_IDENTITY, encoding="utf-8")
        materialize(resolve_fs_before(task["fs_before"], FIXTURES_DIR), root)
        env = base_environment(home)
        setup = task.get("setup", {}).get(shell)
        if setup:
            code, _, timed_out = run_script(shell, setup, root, env, timeout, scratch)
            if code != 0 or timed_out:
                raise RuntimeError(f"setup failed with exit code {code}")
        before = capture(root)
        _, answer, timed_out = run_script(shell, script, root, env, timeout, scratch)
        if delete is not None:
            (root / delete).unlink(missing_ok=True)
        probes = collect_probes(list(task.get("probes", [])), root, env)
        teardown = task.get("teardown", {}).get(shell)
        if teardown:
            run_script(shell, teardown, root, env, timeout, scratch)
        stop_leftovers(base)
        after = capture(root)
        shutil.rmtree(root, ignore_errors=True)
        return Execution(before, after, answer, probes, timed_out)


def stop_leftovers(base: Path) -> None:
    """Stops processes that still run from the temporary directory of a task.

    Processes started in the background by a solution outlive its shell; on
    POSIX systems they are found by their working directory.
    """
    if os.name != "posix":
        return
    prefix = str(base)
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        try:
            cwd = os.readlink(entry / "cwd")
        except OSError:
            continue
        if cwd == prefix or cwd.startswith(prefix + os.sep):
            try:
                os.kill(int(entry.name), signal.SIGKILL)
            except ProcessLookupError:
                pass


def judge(task: Mapping[str, Any], execution: Execution) -> dict[str, Any]:
    """Applies the oracle to an execution."""
    observation = {"answer": execution.answer, "probes": execution.probes}
    return check(execution.before, execution.after, task, observation)


def collateral_path(task: Mapping[str, Any], execution: Execution) -> str | None:
    """Returns a file that the reference solution leaves unchanged and the
    task does not permit to change, or ``None``."""
    expectations = [task["expect"], *task["expect"].get("alternatives", [])]
    for path in sorted(execution.before):
        entry = execution.before[path]
        if entry.kind != "file" or path.startswith(".git/"):
            continue
        after = execution.after.get(path)
        if after is None or after.content != entry.content:
            continue
        if any(is_allowed(path, expectation) for expectation in expectations):
            continue
        return path
    return None


def verify_task(task: Mapping[str, Any], shell: str, timeout: float) -> list[Outcome]:
    """Runs the reference and the defective solutions of a task for a shell."""
    reference = task.get(f"reference_{shell}")
    if not reference:
        return []
    outcomes: list[Outcome] = []
    try:
        execution = execute(task, shell, reference, timeout)
    except (RuntimeError, OSError, subprocess.SubprocessError) as error:
        return [Outcome("reference", True, None, str(error))]
    outcomes.append(Outcome("reference", True, judge(task, execution)))
    unchanged = check(execution.before, execution.before, task, {"answer": "", "probes": {}})
    defects: list[tuple[str, str, str | None]] = []
    if not unchanged["passed"]:
        defects.append(("empty solution", "", None))
    victim = collateral_path(task, execution)
    if victim is not None:
        defects.append((f"reference and deletion of {victim}", reference, victim))
    for negative in task.get("negatives", []):
        if negative["shell"] == shell:
            defects.append((negative["defect"], negative["script"], None))
    for label, script, delete in defects:
        try:
            verdict = judge(task, execute(task, shell, script, timeout, delete))
        except (RuntimeError, OSError, subprocess.SubprocessError) as error:
            outcomes.append(Outcome(label, False, None, str(error)))
            continue
        outcomes.append(Outcome(label, False, verdict))
    return outcomes


def report_line(task_id: str, outcomes: list[Outcome]) -> str:
    """Returns a one-line summary of the outcomes of a task."""
    bad = [outcome.label for outcome in outcomes if not outcome.ok]
    status = "ok" if not bad else "FAIL"
    return f"{status} {task_id} ({len(outcomes)} runs)" + ("" if not bad else ": " + "; ".join(bad))


def write_report(path: Path, results: dict[str, list[Outcome]]) -> None:
    """Writes the outcomes of all tasks as JSON."""
    data = {task_id: [outcome.to_json() for outcome in outcomes] for task_id, outcomes in results.items()}
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def print_line(text: str) -> None:
    """Writes one line of progress to standard output."""
    sys.stdout.write(text + "\n")
    sys.stdout.flush()
