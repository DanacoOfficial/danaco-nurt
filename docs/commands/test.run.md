---
command: test.run
group: verify
summary: Run the tests of a project and return typed results with a fingerprint of the code state.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# test.run

## Synopsis

```text
test.run [<path>] [name: value]...
```

## Description

Runs the test framework of the project in `path` and returns one `test_run`
header and one `test_result` record per test. The framework is started as a
program without a shell. Its machine-readable report is written to the state
directory, not to the project.

With `framework: auto` the framework is detected from the files of the
project in this order: `bun` (`bunfig.toml`, `bun.lock`, a `test` script that
runs `bun test`), `vitest`, `jest`, `npm` (a `test` script other than the
placeholder written by `npm init`), `pytest`, `go` (`go.mod`) and `cargo`
(`Cargo.toml`). A directory that holds `*.test.*` or `*.spec.*` files and no
other marker is run with `bun`. `command` replaces the command of the
framework; without `framework` its output is read as TAP and as lines marked
`(pass)`, `(fail)`, `(skip)` or `(todo)`.

`targets` and `filter` are translated to the options of the framework and
`bail` to its fail-fast option. A target that starts with `-` is rejected;
options of the framework are given through `command`. The `npm` framework is
recorded as `custom` in the `test_run`.

The identifier of a test is stable between runs: the file, the suite and the
name joined by `::`. A repeated identifier receives the suffix `#2`, `#3`.
The run is compared with the earlier runs of the same directory. The header
then carries `baseline` (the run with the most shared tests), `regressions`
(tests that passed and now fail) and `fixed` (tests that failed and now
pass), each only when it applies. With `slowest: n` the header carries
`slowest`, the `n` slowest tests as `id: <time>ms`.

`failures` limits how many failed tests keep their `message`, `stack`,
`stdout` and `stderr` in the output; `message_lines` cuts each of them to
that number of lines. The stored run keeps the complete texts.

With `coverage: true` the framework measures coverage: `--coverage` for bun,
vitest and jest, `pytest-cov` for pytest, `-coverprofile` for go and
`cargo llvm-cov` instead of `cargo test` for cargo. `coverage_file` names a
report that the tests produce themselves; it is read after the run and its
format (LCOV, Cobertura, Istanbul, Go, LLVM) is detected from its content.
The result is the `coverage` field of the `test_run`.

The `fingerprint` of the header is `sha256:` and the digest of the sorted list
of file paths and content hashes. In a git repository the files are the
tracked files and the changed untracked files; otherwise they are the files
found under the ignore rules of the directory. Names in `run.preimage_exclude`
are skipped. At most 20 000 files are covered.

When the run is still going after `wait`, it continues as a job and the
command returns the `job` record. The test run is stored when the job ends.

## Parameters

| Parameter | Type | Default | Description |
|---|---|---|---|
| `path` | `path` | none | Project directory. |
| `framework` | `enum(auto\|bun\|vitest\|jest\|npm\|pytest\|go\|cargo)` | `auto` | Test framework. |
| `targets` | `list<text>` | none | Test files, packages or identifiers to run. |
| `filter` | `text` | none | Only tests whose name matches this filter. |
| `bail` | `bool` | `false` | Stop at the first failing test. |
| `command` | `list<text>` | none | Command run instead of the detected one. |
| `coverage` | `bool` | `false` | Collect code coverage. |
| `coverage_file` | `path` | none | Coverage report produced by the tests. |
| `timeout` | `duration` | `30m` | Time limit of the run. |
| `wait` | `duration` | `3s` | Time to wait before the run continues as a job. |
| `slowest` | `int` | `0` | Number of slowest tests reported. |
| `failures` | `int` | `10` | Failed tests reported in detail. |
| `message_lines` | `int` | `5` | Message lines per failed test. |

## Input and output

| Form | Input | Output |
|---|---|---|
| default | `none` | `test_run`, `test_result`, `job` |

The command takes no input. It produces a `test_run` header followed by
`test_result` records, or a `job` record when the run continues in the
background.

The status of the run is `passed`, `failed`, `error` (the framework did not
start or the code did not compile), `timed_out` or `no_tests`. When the run
ends with an error and no test results, a result with the identifier `(run)`
carries the end of the output. A test file that cannot be loaded and a Go
package that does not build are reported as results with the status `error`.

## Effect

| Property | Value |
|---|---|
| Category | `run` |
| Effect | `process` |
| Reversibility | `reversible_if_preserved` |
| Implementation | `user_program` |
| Reversibility conditions | Files changed by the test programs are reversible when their previous content was preserved; effects outside the file system are irreversible. |

## Examples

Run the tests of the current project and show only the failed ones.

```nurt example
test.run |> filter status == "failed"
```

Run one file with the first failure stopping the run.

```nurt example
test.run framework: bun targets: ["src/sum.test.ts"] bail: true
```

Run a Go project with coverage and show the coverage of the run.

```nurt example
test.run framework: go coverage: true |> select coverage
```

## Errors

| Code | Meaning | Typical fix |
|---|---|---|
| `E_TEST_NO_FRAMEWORK` | No framework was recognised. | Name the framework or give `command:`. |
| `E_EXTERNAL_MISSING` | The test program is not installed. | Install it or give `command:`. |
| `E_OUTPUT_PARSE` | The report or the output cannot be read. | Run with `command:` to see the output; the error carries the execution record. |
| `E_INVALID_VALUE` | A target looks like an option. | Pass options through `command:`. |

## Related

`test.failed`, `test.show`, `test.list`, `run`, `jobs`.
