---
command: run
group: core
summary: Run a program without a shell; after wait it continues as a background job.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# run

## Synopsis

```text
run <command> cwd: <path> [name: value]...
```

## Description

`run` starts a program with the arguments of `command` as a list. No shell
takes part: the arguments reach the program exactly as written, and shell
syntax such as `|`, `>`, `*` or `$NAME` has no meaning. A shell script runs as
a program of its own, for example `run ["bash", "-c", "make && make install"]`.

The first element of `command` names the program. A name with a directory,
such as `./tool` or `/usr/bin/tool`, is taken relative to `cwd`. Any other name
is looked up in `PATH` of the environment the program receives, so a `PATH`
given in `env` takes effect; empty entries of `PATH` are skipped. On Windows
the extensions of `PATHEXT` are tried in order after the name as written, and
variable names are compared without case.

On Windows, batch files (`.bat`, `.cmd`) are started through
`%SystemRoot%\System32\cmd.exe /e:ON /v:OFF /d /c`, with every argument quoted
for the command interpreter: arguments with spaces or metacharacters
(`& | < > ^ ( ) ! "`) are enclosed in quotes, `"` is doubled and `%` is written
so that `%NAME%` is not expanded. The interpreter is never looked up in
`PATH`. The static check lists the arguments with metacharacters in
`metacharacter_arguments`.

The environment is built in three layers, each overriding the earlier ones:
the environment of the console (`inherit_env: all`), the table
`run.env_defaults`, and `env`. A variable of `env` with the value `null` is
removed; an empty text sets an empty value. With `inherit_env: none` the
program receives only `run.env_defaults` and `env`; on Windows `SystemRoot`,
`ComSpec` and `PATHEXT` are kept as well, because programs fail to start
without them. A program given by name is looked up in the `PATH` of that
environment, so on Unix it needs `PATH` in `env` (or in `run.env_defaults`)
with `inherit_env: none`, otherwise the result is `E_PROGRAM_NOT_FOUND`. The
result lists the names given in `env` in `env_keys`; values
never appear in the result or in the journal. The static check reports the
variables of `run.env_defaults` in `env_defaults`.

There is no terminal: standard input is empty (`/dev/null`, `NUL`) unless
`stdin` gives a text, written to the program and then closed, or a path,
opened as the input of the program. When the program closes its input before
all of the text is written, the rest is discarded and the report notes
`stdin_closed_early`. The program runs in its own process group (Windows: job
object); `timeout`, `idle_timeout`, `max_cpu` and `max_memory` apply to the
whole process tree.

Output processing. The full output is kept in spool files; the parameters
below shape only the response, which is short by default. Each line is
decoded (`output_encoding`), cleaned (terminal control sequences, carriage
return redraws, which keep only the last state of the line, and trailing white
space are removed), checked against `summary` and `progress`, collapsed with
repeats when `dedupe` is set (the default: consecutive lines that are
identical, or that differ only in numbers, versions or the names after a step
word such as `Compiling`, become the first of them followed by ` (×N)`),
selected by `filter` with `context` lines around each match (or, for `show:
errors` and for `show: auto` after a failure of output longer than 20 lines,
by the built-in error line patterns with 2 lines of context), limited to the
first `head` and the last `tail` lines, and cut to 400 characters per line
(the rest becomes `… [+N characters]`). The text of all streams together then
fits in `max_bytes` bytes (4096 by default, key `output.max_bytes`, `0` is no
limit), filled in this order: the errors found in the output of a known tool,
the end of standard error, the end of standard output, the beginning of the
output; the end has precedence over the beginning. `quiet_on_success` and
`show: none` return no text but keep the counters, `summary_lines` and
`progress`. `shown` reports, per stream, the lines kept from the start and
from the end, the lines left out between them and the lines removed by the
filters.

With `show: auto` and without `head`, `tail` and `filter`, a run that exits
with 0 returns the last 5 lines of its output and the field `digest`, one line
with the exit code, the time, and the lines and bytes of output (for a known
tool also its counters, for example `cargo: 0 errors, 3 warnings`). A failed
run returns the full budget.

Known formats. After a failure of output longer than 20 lines, `show: auto`
recognises the output of `cargo` and `rustc` (`error[E0308]` with
`--> file:line:col`) and of `pytest` (`FAILED file::test - message` with the
traceback line). The field `diagnostics` then has `tool`, `errors` and
`warnings` (the numbers of all of them) and `first`, the first 10 distinct
errors, each with the one-line fields `file`, `line`, `col`, `code`, `message`
and `severity` (and `count` when the same error was reported in several
places); the text keeps only the last 10 lines of the output instead of
hundreds of warnings. A shorter output is returned whole and has no
`diagnostics`. A line `error: message` without a code and a location counts as
an error of `cargo` only when the output shows another mark of it.

When the response leaves lines out or cuts lines, `read_hint` is one sentence
with ready commands that read the rest from the output file without running
the program again, for example `read "/state/spool/r1.out" lines: 200..260`,
and `search /error|FAILED/ "/state/spool/r1.out"` for the lines that contain
a word. Line numbers are those of the file, so lines collapsed by `dedupe`
count as the lines they stand for.

`run` waits up to `wait` for the program. A program that ends within `wait`
yields an `exec` record. A program still running after `wait` continues as a
background job and the result is a `job` record with `state: running`; the
stages after the effect are then skipped and the report notes
`post_stages_deferred`. `background: true` is the same as `wait: 0s`;
`wait: forever` waits until `timeout`. Parameters the program does not give
take their values from the configuration keys `run.timeout`, `run.wait`,
`run.idle_timeout`, `run.track_changes`, `run.parallel`, `output.head`,
`output.tail` and `output.max_bytes`.

Changes made by the program are detected with `track_changes`. The mode
`auto` is bounded so that it never stalls a run: it does not scan the file
system root, does not enter directories of other file systems, and gives up
when the watched directories hold more than 200 000 entries or one scan takes
longer than 3 s. The program then runs without detection and the report
carries a `detection_scope` note that gives the reason and says what to do:
name the directories the program changes in `watch`, or use
`track_changes: scan`. A tracked run carries a `detection_scope` note that
names the watched directories and the directories of other file systems that
were not entered. The scans before and after the program end at cancellation
and at `timeout`; a scan before the program that reaches `timeout` fails the
run with `E_TIMEOUT` and one that is cancelled with `E_CANCELLED`. The scan
after the program may exceed `timeout` by up to 5 s, so that the changes of a
program ended by the limit are still recorded; a scan that stops early records
no changes and says so in a note. A `run` that changes no file does not report
`nothing_matched`, because the program is not a stage that selects targets.

An exit code other than zero is a result, not an error: the status is `ok`,
`exec.exit_code` holds the code and the report notes `exit_nonzero`. Use
`|> assert exit_code: 0` to fail on it. A program that the console ends
because of `timeout` or `idle_timeout` fails with `E_TIMEOUT`, one ended by
`max_cpu` or `max_memory` with `E_LIMIT`, and one ended by cancellation with
`E_CANCELLED`; the error carries the `exec` record in `details.exec` and
`effect_applied: true`, because the changes made until then remain.

With input records, the `path` of every `entry` or `match` record becomes an
argument after `command`. Without `batch` one process runs per record; with
`batch: n` the arguments are split into batches of up to `n`. `parallel` runs
that many batches at once; results keep the batch order. All batches run to
their end before the call returns, whatever `wait` says. The result is one
`exec` record: its `exit_code` is 0 when every batch exited with 0, otherwise
the code of the first failed batch in batch order; `batches` lists the number,
arguments, exit code and running time of every batch and `failed_batches`
counts the failures. After a failure the remaining batches still run, unless
`bail: true`, which starts no further batch. An empty input starts no
process. When `command` uses fields of the input record, the command is
computed per record and `batch` is not accepted.

A preview (`preview: true`) resolves the program and returns the program, the
arguments as the program receives them (`argv`; for batch files the command
line of the interpreter), the working directory, `launcher`, `env_keys`,
`stdin`, `timeout`, `wait`, `watch` and `track_changes`, without starting
anything.

The command line of a batch file cannot hold a line break: the interpreter
ends the command there. An argument with `\r` or `\n` for a batch file, and an
argument with a NUL character for any program, fails with `E_ARG`;
`details.index` gives the position in `command` and `fix` passes the text on
standard input instead.

Every job of a run, a batch or a `retry` emits `job.started`, then
`job.output` and `job.progress`, then one `job.finished`, also when the job
continues in the background. The command emits no `exec.started` or
`exec.finished`: these events enclose the whole program and come from the
engine.

## Privilege

`privilege: elevated` runs the program with the rights of the administrator, `as_user: "NAME"` as another account; `sudo`, `doas` and `run0` are not written in the argument list (a list that starts with `sudo` is corrected into the same `run` with `privilege: elevated`). The files that the program changes are reported, but they are not preserved: the entry is irreversible, and the report says so before the change.

```nurt
run ["systemctl", "daemon-reload"] cwd: "/" privilege: elevated
```

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `command` | `list<text>` | required | Program and arguments; the first element is the program. |
| `cwd` | `path` | required | Working directory of the program. |
| `env` | `map<text>` | none | Environment variables to set. |
| `inherit_env` | `enum(all\|none)` | `all` | Inherit the environment of the console or start with an empty one. |
| `stdin` | `text \| path` | none | Text or file given on standard input; the input is empty otherwise. |
| `timeout` | `duration` | `120s` | Time limit of the program. |
| `idle_timeout` | `duration` | `60s` | Time limit without output. |
| `wait` | `duration \| enum(forever)` | `3s` | Time to wait for the program before it continues as a job; forever waits until timeout. |
| `background` | `bool` | `false` | Start as a job at once, the same as wait: 0s. |
| `merge_stderr` | `bool` | `false` | Merge standard error into standard output. |
| `show` | `enum(auto\|both\|stdout\|stderr\|errors\|summary\|none)` | `auto` | Output returned in the result; the full output is always kept in a file. |
| `head` | `int` | `20` | Number of first output lines returned. |
| `tail` | `int` | `40` | Number of last output lines returned. |
| `max_bytes` | `int` | `4096` | Bytes of output text returned in all streams together; 0 is no limit. |
| `filter` | `regex` | none | Return only output lines that match. |
| `context` | `int` | `0` | Lines of context around lines matching filter. |
| `dedupe` | `bool` | `true` | Collapse repeated output lines, also lines that differ only in numbers. |
| `quiet_on_success` | `bool` | `false` | Return no output when the exit code is 0. |
| `output_encoding` | `text` | `"auto"` | Encoding of the program output; auto detects it. |
| `watch` | `list<path>` | none | Directories in which changes made by the program are detected. |
| `track_changes` | `enum(auto\|scan\|events\|none)` | `auto` | Method of detecting changes made by the program. |
| `max_cpu` | `duration` | none | Limit of processor time. |
| `max_memory` | `size` | none | Limit of memory. |
| `priority` | `enum(low\|normal\|high)` | `normal` | Scheduling priority. |
| `label` | `text` | none | Short name of the job. |
| `description` | `text` | none | Description of the job. |
| `progress` | `regex` | none | Pattern of output lines that report progress. |
| `summary` | `list<regex>` | none | Patterns whose latest matches form the status line. |
| `batch` | `int` | none | Run the program once per batch of this many input arguments. |
| `parallel` | `int` | `1` | Number of batches run at the same time; results keep the batch order. |
| `bail` | `bool` | `false` | Stop starting batches after the first failed batch. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none`, `entry`, `match` | `exec`, `job` |

## Effect

| Property | Value |
|---|---|
| Category | `run` |
| Effect | `process` |
| Reversibility | `reversible_if_preserved` |
| Implementation | `user_program` |
| Reversibility conditions | Changes of files under watch are reversible when their previous content was kept; effects outside the file system are irreversible. |

Changes of files under `watch` are recorded in the journal and are
reversible when their previous content was kept; effects outside the file
system are irreversible. A run that changes no file creates no journal entry.

A command line that matches the built-in table of programs with effects
outside the computer (for example `git push` or `kubectl apply`, also behind
wrappers such as `sudo` or `env`) or a pattern of `run.irreversible_patterns`
lists the matched rules in `exec.external_effect`, and the report marks the
call `irreversible: external_effect`. The classification describes the
effect; it never prevents the run.

## Examples

Run the tests of a project and show only the error lines:

```nurt example
run ["cargo", "test"] cwd: "/home/user/app" show: errors head: 5 tail: 10
```

Start a build in the background with a label:

```nurt example
run ["npm", "run", "build"] cwd: "/home/user/app" background: true label: "build"
```

Format the Go files found in a project, 20 files per process, two processes
at a time:

```nurt example
find "/home/user/app" glob: "**/*.go" |> run ["gofmt", "-w"] batch: 20 parallel: 2 cwd: "/home/user/app"
```

Give text on standard input, with an environment of its own:

```nurt example
run ["sort"] cwd: "/home/user" stdin: "b\na\n" inherit_env: none env: {"LC_ALL": "C", "PATH": "/usr/bin:/bin"}
```

Fail when the program exits with a code other than zero:

```nurt example
run ["go", "build", "./..."] cwd: "/home/user/app" |> assert exit_code: 0
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_PROGRAM_NOT_FOUND` | The program is not in `PATH` or the named file does not exist; `candidates` lists up to five similar names of the search path. | Accept a candidate, give the full path, or add the directory to `PATH` in `env`. |
| `E_SPAWN` | The program could not be started: the working directory is missing or not a directory, the file is a directory or has no execute permission, or the format is unknown. | Give an existing `cwd`; grant execute permission or start the file through its interpreter. |
| `E_ARG` | An argument or variable cannot be passed without changing its meaning: a NUL character, a line break in an argument of a batch file, `=` in a variable name. | Use the `fix`, which passes the text on standard input. |
| `E_MISSING_PARAM` | Neither `cwd` nor `base` is given. | Add `cwd`. |
| `E_PATH_RELATIVE` | `cwd` is relative and `base` is not given. | Give an absolute `cwd`. |
| `E_INVALID_VALUE` | `batch` or `parallel` is below 1, or `batch` is used with a `command` computed per record. | Give a count of 1 or more; remove `batch`. |
| `E_NOT_FOUND` | The `stdin` file does not exist. | Give an existing file or a text. |
| `E_TIMEOUT` | The program reached `timeout` or `idle_timeout`; `details.exec` holds the record. | Raise the limit or start the program with `background: true`. |
| `E_LIMIT` | The program exceeded `max_cpu` or `max_memory`. | Raise the limit. |
| `E_CANCELLED` | The call was cancelled while the program ran. | Run the call again. |

## Related

`jobs`, `stop`, `retry`, `suspend`, `resume`, `assert`, `verify`, `undo`,
`test.run`, `analyze.check`.
