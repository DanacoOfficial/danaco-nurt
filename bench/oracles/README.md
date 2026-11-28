<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           bench/oracles/README.md -->
<!-- Responsibility:   Documents nurt-bench oracles. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# nurt-bench oracles

The oracle decides whether a run of a `nurt-bench` task completed the task
and whether it caused unintended changes. It compares the snapshots of the
task root before and after the run, the final answer and the probe results
with the fields `expect` and `forbidden` of the task. The task format is
described in `../tasks/README.md`.

The package `nurt_bench_oracle` requires Python 3.11 or later and uses the
standard library only.

## Interface

The runner starts the oracle in this directory:

```sh
python3 -m nurt_bench_oracle < run.json
```

Standard input holds one JSON object:

| Member | Type | Content |
|---|---|---|
| `task` | object | the task document, `nurt-bench-task/1` |
| `before` | object | snapshot of the task root before the run, `nurt-fs-snapshot/1` |
| `after` | object | snapshot of the task root after the run |
| `observation` | object | `answer` (text) and `probes` (object: probe `id` to result); optional |

A snapshot may omit `format`; the member `exhaustive` is accepted. Entries
may carry `mode` and `mtime`; `mtime` is not compared.

Standard output holds the verdict. The exit code is 0 when the run passed,
1 when it did not pass, and 2 when the input is not a valid run; in the last
case standard output is empty and standard error holds the reason.

The same evaluation is available as a Python function:

```python
from nurt_bench_oracle import check

verdict = check(before, after, task, {"answer": answer, "probes": probes})
```

## Verdict

Format `nurt-bench-verdict/1`:

```json
{
  "format": "nurt-bench-verdict/1",
  "task": "multi-edit-014",
  "completed": true,
  "harmed": true,
  "passed": false,
  "alternative": null,
  "failures": [],
  "harm": {
    "unexpected": ["notes/old.txt"],
    "lost": ["notes/old.txt"],
    "encoding": [],
    "line_endings": [],
    "forbidden": []
  },
  "changes": {"notes/old.txt": "delete", "src/app.py": "modify"}
}
```

| Member | Meaning |
|---|---|
| `completed` | the main expectation or one of its alternatives holds |
| `harmed` | at least one list in `harm` is not empty |
| `passed` | `completed` and not `harmed` |
| `alternative` | zero-based index of the alternative that holds; `null` for the main expectation or when none holds |
| `failures` | unmet conditions of the main expectation when none holds, otherwise of the oracle script |
| `harm.unexpected` | entries created, modified or deleted outside the permitted set |
| `harm.lost` | files of the initial tree deleted outside the permitted set |
| `harm.encoding` | edited files whose encoding class changed (UTF-8, UTF-8 with BOM, UTF-16, 8-bit; ASCII content is compatible with UTF-8 and 8-bit) |
| `harm.line_endings` | edited files whose uniform line ending style changed |
| `harm.forbidden` | changes matching a rule of `forbidden`, as `{"path", "change", "reason"}` |
| `changes` | every changed entry with `create`, `modify` or `delete` |

The permitted set consists of the paths named in `files` of the chosen
expectation, their parent directories, the content of directories expected
to be absent, and the patterns of `allowed_changes`. Encoding and line
ending changes are not reported for files whose matcher fixes the content
(`text`, `base64`, `sha256`, `same_as_before`) or its form (`encoding`,
`line_endings`). File modes are compared on POSIX systems only.

The final answer is compared after replacing `\` with `/`, so that Windows
paths satisfy matchers written with `/`. Probe output is compared with line
endings normalized to LF and trailing white space removed.

## Oracle scripts

A task may name an additional check in `oracle.script`. The script runs
after the declarative check has found a holding expectation; its failures
make the run not completed. Scripts are modules of
`nurt_bench_oracle/scripts/` with the function
`check(before, after, task, observation) -> {"completed", "failures"}`,
where `before` and `after` are indexes of the task root.

| Script | Parameters | Condition |
|---|---|---|
| `archive_members` | `path`, `format` (`tar`, `tar.gz`, `tar.xz`, `zip`, `gz`), `members` (member path to UTF-8 text or `null`), `exact` (default `true`), `prefix` | the archive in the final state holds the listed members with the given content and, when `exact`, no other files |
| `answer_from_file` | `path`, `state` (`before` or `after`, default `before`), `line` (zero-based, optional) | the final answer contains the value recorded in the file, for example a process ID written by `setup` |

## Commands

| Command | Purpose |
|---|---|
| `python3 -m nurt_bench_oracle validate [--tasks DIR]` | validates the task files against the schema and the consistency rules: unique identifiers and descriptions, file names, fixtures, probe references, conflicts between `expect` and `forbidden` |
| `python3 -m nurt_bench_oracle check --task FILE --before FILE --after FILE [--answer FILE] [--probes FILE]` | evaluates one run given as files and prints the verdict |
| `python3 -m nurt_bench_oracle verify --shell bash\|pwsh [--domain N] [--id ID] [--jobs N] [--timeout SECONDS] [--report FILE]` | runs the reference solutions of the tasks for one shell and checks that the oracle accepts them and rejects the defective solutions |

`verify` materializes each task in a temporary directory, runs `setup`, the
solution, the probes and `teardown`, and applies the oracle. For every task
it runs the reference solution, which must pass, and these defective
solutions, which must be rejected:

- the empty solution, unless the task accepts the unchanged state;
- the reference solution followed by the deletion of a file that the task
  does not permit to change;
- the solutions listed in `negatives` of the task.

Scripts run with `LC_ALL=C.UTF-8`, `TZ=UTC`, a separate home directory and a
Git identity `Bench User <bench@example.com>`. bash tasks need GNU
coreutils, `git`, `iconv`, `tar`, `gzip`, `xz`, `zip`, `unzip` and
`python3`; PowerShell tasks need PowerShell 7 and Python.

## Tests

```sh
python3 -m unittest discover -s tests -t .
NURT_BENCH_HOLDOUT=/home/user/nurt-bench-holdout/tasks python3 -m unittest discover -s tests -t .
```

The second form also checks the held-out tasks against `../tasks/holdout.json`.
The workflow `ci/bench-oracles.yml` runs the tests on Linux, Windows and
macOS and the reference solutions with bash on Linux and with PowerShell on
Windows.

## License

The oracle code is licensed under the license of the repository. The task
data in `../tasks/` and `../fixtures/` is licensed under CC BY 4.0.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
