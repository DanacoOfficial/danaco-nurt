<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Danaco Nurt conformance suite

The conformance suite checks that an implementation of the Nurt language
behaves as the specification requires. The suite talks to the implementation
only through the server protocol `nurt-rpc/1` of `nurt serve --stdio`, so an
implementation written in any language and without any code of this
repository can be tested. An implementation or integration that passes every
mandatory case of a released version of the suite meets the technical
requirement of the "Danaco Nurt Compatible" program.

The runner is the program `nurt-conformance` of the crate
`crates/nurt-conformance`. The cases are data files in this directory.

## Running the suite

```
nurt-conformance run --impl "<command that starts the server>" [options]
```

| Option | Type | Default | Meaning |
|---|---|---|---|
| `--impl` | text | required | Command line that starts the server and speaks `nurt-rpc/1` on standard input and output, for example `"nurt serve --stdio"`. Words are separated by white space; single and double quotes group words. |
| `--cases` | path | `conformance` | Root directory of the cases. |
| `--filter` | text, repeatable | none | Runs only cases whose identifier contains one of the given texts, or whose identifier matches the pattern when the text contains `*`. |
| `--kind` | text, repeatable | all | Runs only cases of the given kinds. |
| `--jobs` | integer | number of processors | Number of cases run in parallel. |
| `--startup-timeout` | duration | `30s` | Time limit of the start of a server process and of its answer to `initialize`. |
| `--timeout` | duration | `30s` | Time limit of one case, counted from the answer to `initialize`. |
| `--report` | path | none | Writes the result in the format `nurt-conformance-report/1`. |
| `--quiet` | flag | off | Prints only the summary. |
| `--second-volume` | path | `NURT_CONFORMANCE_SECOND_VOLUME` | Directory on another file system than the temporary directory, for the cases that require `fixture:second-volume`, see "Fixtures". Without it these cases are skipped. |

Each case runs in a new server process. Before the start the runner creates a
temporary directory with three subdirectories and passes them to the server:

| Directory | Passed as | Content |
|---|---|---|
| `work` | `options.base` of every `run` and `check` request; placeholder `${base}` | the file system state of the case |
| `state` | environment variable `NURT_STATE_DIR` | journal and other state of the instance |
| `config` | environment variable `NURT_CONFIG` (path of `config/config.toml`) | configuration of the case with its placeholders replaced (see "Placeholders"); empty when the case has none |

A case that requires fixtures also gets the fixtures described in "Fixtures".

Exit codes: `0` when every applicable mandatory case passed, `1` when at least
one mandatory case failed, `2` for an invalid invocation or an unreadable case
directory.

Example on Linux:

```
nurt-conformance run --impl "nurt serve --stdio" --report /home/user/nurt-conformance.json
```

## Server requirements

The runner sends one JSON-RPC 2.0 message per line (NDJSON, UTF-8) and reads
one message per line from standard output. The first call is `initialize`;
the last is `shutdown`, followed by the end of standard input.

| Method | Parameters | Result used by the suite |
|---|---|---|
| `initialize` | `protocol` (`"nurt-rpc/1"`), `client` (`name`, `version`), `capabilities` (`events`) | `protocol`, `version`, `language`, `features` (list of feature names) |
| `info` | none | object describing the implementation; copied into the conformance report |
| `parse` | `program`, `variant` | `ast` (surface tree `nurt-ast/1`) or `error` |
| `print` | `ast`, `variant` | `text` |
| `check` | request `nurt-request/1` | `bound` (bound tree `nurt-bound/1`), `ast_hash`, or `error`; no effect on the world |
| `correct` | `program`, `variant` | correction result |
| `run` | request `nurt-request/1` | response `nurt-response/1` |
| `page` | `cursor`, `limit` | response `nurt-response/1` |
| `schema` | `which` | JSON Schema document |
| `describe`, `help`, `catalog`, `history`, `space` | as named by the cases | as named by the cases |
| `cancel` | `request_id` or `job_id` | empty object |
| `shutdown` | none | empty object |

Errors of the language are part of the result (`error`, or `report.error` of a
response). JSON-RPC errors are reserved for protocol and call errors: the codes
`-32700`, `-32600`, `-32601`, `-32602` and `-32603`, and `-32000 - status` for
a status of the C interface (`NURT_E_ARG` is `-32003`), with the name of the
status in `error.data.status`. Events are notifications `nurt/event` and are
sent only after `initialize` with `capabilities.events` set to `true`. The
events of a request are sent before its response: the runner attributes to a
call the events received between the request and the response.

## Case classes

| Class | Meaning |
|---|---|
| `mandatory` | Required for conformance. |
| `optional:<feature>` | Run only when the implementation lists `<feature>` in `features`; otherwise reported as skipped. A failed optional case does not affect conformance. |

A case may also list conditions in `requires`: `os:linux`, `os:macos`,
`os:windows`, `os:unix`, `feature:<name>`, and the fixtures `fixture:http` and
`fixture:second-volume` (see "Fixtures"). A case whose conditions do not hold
on the current system, implementation or run, or whose language major version
differs from that of the implementation, is reported as skipped. Skipped cases
do not affect conformance.

### Feature `testkit`

An implementation that lists the feature `testkit` offers test methods:

| Method | Parameters | Result |
|---|---|---|
| `testkit.failpoint.set` | `stage` (`parse`, `bind`, `read`, `plan`, `apply`, `verify`), `code` (an error code) | empty object; the named stage of the next call fails with that code |
| `testkit.clock.advance` | `by`: duration, an integer followed by `ms`, `s`, `m`, `h` or `d`, for example `"2h"` | `now`: time of the clock after the advance, RFC 3339 in UTC |

`testkit.clock.advance` moves the clock of the instance; the times that the
implementation measures, such as the age of journal entries and the times of
events, follow that clock. The optional cases of `errors/injected` use
failpoints for errors that depend on the state of the system, of remote hosts
or of external programs. Mandatory cases never require the feature.

An implementation may offer only some of these methods. A case whose call of a
method `testkit.*` is answered with the error `-32601` (method not found)
before any check failed is reported as skipped with the reason
`the implementation does not offer <method>`; after a failed check the case
fails. A step that gives `expect_error = -32601` for such a call is checked as
any other step.

## Case kinds and directories

| Kind | Directory | Content |
|---|---|---|
| `golden` | `golden/` | program text in the variants A, B and C against surface and bound trees, the canonical printer and `ast_hash`; published schemas |
| `bind` | `bind/` | binding: explicit defaults, expansion of short forms, phases after the effect, `E_ONE_EFFECT`, `check` without effect |
| `error` | `errors/` | every error code of the language |
| `exec` | `exec/` | execution of commands on a file system state |
| `journal` | `journal/` | journal, undo and redo |
| `correction` | `correction/` | correction of program text |
| `parity` | `parity/` | results equal to those of established tools |
| `binding` | `bindings/` | application interface of the language bindings (separate format, see below) |

## Case format

A case is either a directory with the file `case.toml` or an entry of a
collection file. The schema of both is
[`schema/conformance-case-1.schema.json`](schema/conformance-case-1.schema.json);
the TOML documents are validated in their JSON form.

### Case file `case.toml`

| Key | Type | Default | Meaning |
|---|---|---|---|
| `id` | text | required | Identifier, unique in the suite: lower case letters, digits and `. _ / -`. |
| `kind` | text | required | Kind from the table above. |
| `class` | text | `"mandatory"` | `mandatory` or `optional:<feature>`. |
| `language` | text | `"1.0"` | Version of the language the case is written for, `MAJOR.MINOR`. A case whose major version differs from `language` of the implementation is skipped. |
| `requires` | list of text | `[]` | Conditions, see "Case classes". |
| `summary` | text | required | One sentence describing the checked behavior. |
| `variant` | text | `"a"` | Default syntax variant of the programs of the case. |
| `code` | text | none | Error code the case covers; required for kind `error`. |
| `volatile` | list of text | `[]` | Further JSON pointers of volatile values (see "Comparison"). |
| `config` | text | none | File with the configuration of the case, written to `config/config.toml`. |
| `fs_before` | text or table | file `fs-before.json` when present | Initial state of `work`, a snapshot `nurt-fs-snapshot/1`. |
| `second_volume_before` | text or table | none | Initial state of the directory of the case on the second volume, a snapshot; requires `fixture:second-volume`. |
| `http_files` | text or table | none | Snapshot whose files the local HTTP server serves below `/files/`; requires `fixture:http`. |
| `step` | list of tables | derived from the files | Calls of the case, see "Steps". |

A case of one call may give the keys of the step (see "Steps") directly in
`case.toml` instead of a list `step`. A text value of `config`, `fs_before`,
`second_volume_before`, `http_files`, `options`, `params`, `expect`,
`fs_after` and `second_volume_after` names a file of the case directory; a
table is the value itself.

### Files of a case directory

When a case has neither `step` nor the keys of a step, its calls are derived
from the files of the directory:

| Kind | Files | Calls |
|---|---|---|
| `golden` | `a.nurt`, `b.nurt`, `c.nurt` (each optional), `ast.json` (required), `ast.hash`, `c.json`, `c.hash`, `bound.json`, `bound.hash` | see "Golden cases" |
| `bind` | `program.nurt`, `bound.json` and/or `expect.json` | `check` of the program; the bound tree is compared as a tree, `expect.json` as a subset of the result; `work` must be unchanged |
| `error`, `exec`, `journal` | `program.nurt` or `request.json`, `expect.json` or `report.json`, `fs-after.json` | `run` of the program or request; `expect.json` is compared with the response, `report.json` with its member `report`; `fs-after.json` with `work` after the call |

### Golden cases

The files of a golden case are copies of the golden files of `spec/golden/`
and keep their format:

| File of the case | Golden file |
|---|---|
| `a.nurt`, `b.nurt`, `c.nurt` | `ast/<name>.nurt`, `ast/<name>.b.nurt`, `ast/<name>.c.nurt` |
| `ast.json`, `ast.hash` | `ast/<name>.json`, `ast/<name>.hash` |
| `c.json`, `c.hash` | `ast/<name>.c.json`, `ast/<name>.c.hash` |
| `bound.json`, `bound.hash` | `bound/<name>.json`, `bound/<name>.hash` |

The runner checks:

1. `parse` of `a.nurt` and of `b.nurt` returns a tree that is valid under the
   schema `nurt-ast/1` and equals `ast.json` as a tree.
2. `parse` of `c.nurt` returns a valid tree that equals `c.json` as a tree.
   Variant C writes every argument by name, so its surface tree differs from
   variant A; without `c.json` only its bound tree is compared.
3. When `bound.json` exists: `check` of every variant returns a bound tree
   valid under `nurt-bound/1` that equals `bound.json` as a tree, and the
   returned `ast_hash`, when present, equals the content of `bound.hash`.
4. Canonical printer: `print` of `ast.json` in variants A and B, and of
   `c.json` (or `ast.json` when it is absent) in variant C, returns a text
   whose `parse` in the same variant gives a tree equal to the printed tree;
   in variant C the bound tree of the text also equals `bound.json` when it
   exists.

The documents of `spec/golden/ast/invalid/` are error cases of
`errors/invalid-trees.cases.json`: each case sends the document as the member
`ast` of a `run` request and expects the error code listed for it in
`expected.tsv`. The documents of `spec/golden/bound/invalid/` have no case,
because no method of `nurt-rpc/1` accepts a bound tree.

Two trees are equal as trees when their hash forms are equal: the member
`meta` of the root and every member `span` removed. The hash form is also the
input of `ast_hash`, so the comparison is equivalent to comparing `ast_hash`.

A case without program text (only `ast.json`) checks the printer and the
parser on the tree alone.

### Steps

| Key | Type | Default | Meaning |
|---|---|---|---|
| `method` | text | `"run"` | Method of `nurt-rpc/1`. |
| `program` | text | none | Program text. |
| `program_file` | text | none | File with the program text. |
| `variant` | text | variant of the case | Syntax variant of `program`. |
| `options` | text or table | none | Request options merged into `options` of the request. |
| `params` | text or table | built from the keys above | Complete parameters of the method; replaces `program`, `variant` and `options`. |
| `expect` | text or table | none | Expected result, compared with the part of the result named by `expect_at`. |
| `expect_at` | text | `""` | JSON pointer into the result. |
| `match` | text | `"subset"` | `subset`, `exact` or `tree`, see "Comparison". |
| `expect_error` | integer | none | The call must fail with this JSON-RPC error code. |
| `validate` | text | none | The part of the result named by `expect_at` must be valid under the schema with this name: `ast`, `bound`, `report`, `request`, `config`, `catalog`, `value`, `tool` or `metaschema` (JSON Schema 2020-12). |
| `fs_after` | text or table | none | Expected state of `work` after the call; `"unchanged"` requires the state before the call. |
| `absent` | list of text | `[]` | JSON pointer patterns into the result at which no value may exist, see "Absent members". |
| `events` | list of tables | none | Expected events `nurt/event` of the call, see "Events". |
| `events_match` | text | `"ordered"` | `ordered` or `exact`, see "Events". |
| `second_volume_after` | text or table | none | Expected state of the directory of the case on the second volume after the call; `"unchanged"` requires the state before the call. Requires `fixture:second-volume`. |

For the methods `run` and `check` the runner builds the request
`{"nurt": "1.0", "program": …, "variant": …, "options": {…}}` and sets
`options.base` to the directory `work` unless the options name `base`
themselves.

### Placeholders

Text values of `program`, `options` and `params` may contain:

| Placeholder | Replaced by |
|---|---|
| `${base}` | path of `work`, with `/` as separator |
| `${http_base}` | address of the local HTTP server of the case, `http://127.0.0.1:<port>` without a trailing `/`; requires `fixture:http` |
| `${second_volume}` | path of the directory of the case on the second volume, with `/` as separator; requires `fixture:second-volume` |
| `${stepN/pointer}` | value at the JSON pointer in the result of step `N` (counted from 1) |

A text value that consists of a single placeholder is replaced by the JSON
value itself, which keeps numbers and objects; inside a longer text the value
is inserted as text. Example: `program = "journal.undo op: ${step1/report/op}"`.

The text of the configuration file of a case may contain `${base}`,
`${second_volume}` and `${work_mount}`. `${work_mount}` is the mount point of
the file system that holds `work`, with `/` as separator: on Linux the mount
point of the mount table of the process, on other Unix systems the highest
ancestor directory on the same device, on Windows the root of the drive or
share, for example `C:/`. The values are escaped for a TOML basic string, so a
placeholder is written inside double quotes:

```toml
[journal.volume."${work_mount}"]
max_size = "1mib"
```

Other texts of the configuration are not changed.

### Collection files

Files named `*.cases.toml` or `*.cases.json` hold many cases:

```toml
format = "nurt-conformance-cases/1"

[[case]]
id = "errors/parse/lexical-unterminated-text"
kind = "error"
code = "E_LEXICAL"
summary = "an unterminated text is a lexical error"
program = 'find "/home/user'
expect = { report = { status = "failed", error = { code = "E_LEXICAL" } } }
fs_after = "unchanged"
```

An entry has the keys of `case.toml`. A case of one call may give the keys of
the step (`method`, `program`, `variant`, `options`, `params`, `expect`,
`expect_at`, `match`, `expect_error`, `validate`, `fs_after`, `absent`,
`events`, `events_match`, `second_volume_after`) directly in the entry;
otherwise it lists its calls in `step`. Text values that name files are
resolved against the directory of the collection file.

### Comparison

| Mode | Rule |
|---|---|
| `subset` | Every member of an expected object must be present in the actual object and match; members not listed are ignored. Arrays must have the same length and match element by element. Other values must be equal. |
| `exact` | The values must be equal. |
| `tree` | The hash forms of two syntax trees must be equal. |

Before the comparison the runner normalizes both the expected and the actual
value:

1. Values of members marked `"x-nurt-volatile": true` in the published schema
   of the compared document, and values at the pointers listed in `volatile`
   (`*` matches any member or index), are replaced by the text `"<volatile>"`.
   The member must still be present when the expected value lists it.
2. In the actual value, every text that starts with the path of `work` is
   rewritten to start with `${base}`, every text that starts with the
   directory of the case on the second volume to start with
   `${second_volume}`, and the separators of the rest of the path are written
   as `/`; every occurrence of the address of the local HTTP server is
   replaced by `${http_base}`.

Numbers are equal when their values are equal (`1` equals `1.0`).

### Absent members

Each pattern of `absent` is matched against the result of the call; every
value found is a failure of the check `absent`. A segment `*` matches any
member or index, so `"/report/changes/*/rows_changed"` requires that no change
has the member `rows_changed`.

### Events

The events of a call are the parameters of the notifications `nurt/event`
received after the request and before its response. Before the comparison the
members `seq` and `time` of every received and expected event are replaced by
`"<volatile>"`, and the received events are normalized as results. Each
expected event is compared with a received event as a subset.

| `events_match` | Rule |
|---|---|
| `ordered` | The expected events are received in the listed order; other events may come between them. |
| `exact` | The received events are exactly the expected ones, in the listed order. |

```toml
[[case]]
id = "exec/core/find/events"
kind = "exec"
summary = "a run reports its start before its end"
program = 'find "${base}"'
events = [{ t = "exec.started" }, { t = "exec.finished" }]
```

## Fixtures

A case lists the fixtures it needs in `requires`. The runner provides them to
that case only, in addition to the directories of "Running the suite".

### `fixture:http`

Before the server process of the case starts, the runner starts an HTTP/1.1
server on `127.0.0.1` on a port chosen by the system; it stops the server after
the case. No other host is contacted. `${http_base}` is its address.

| Route | Methods | Answer |
|---|---|---|
| `/files/<path>` | `GET`, `HEAD` | status 200 with the file `<path>` of `http_files`; `content-type` by extension: `.txt` `text/plain; charset=utf-8`, `.json` `application/json`, `.html` `text/html; charset=utf-8`, `.csv` `text/csv; charset=utf-8`, `.xml` `application/xml`, other files `application/octet-stream`; status 404 for a path without a file |
| `/echo`, `/echo/<path>` | any | status 200, `content-type: application/json`, an object describing the request: `method`, `path`, `query` (`null` without a query), `headers` (lower case names; repeated headers joined with `, `) and `body_utf8`, or `body_base64` for a body that is not UTF-8 |
| `/status/<code>` | any | status `<code>` (200 to 599) with an empty body; status 400 for other codes |
| `/bytes/<count>` | `GET` | `<count>` bytes, at most 16 MiB, whose byte number `i` (from 0) has the value `i mod 256`; `content-type: application/octet-stream` |
| `/redirect/<count>` | `GET` | status 302 with `location: /redirect/<count - 1>`; for 0 status 200 with the text `redirected\n` |

The answers also carry the headers that the server adds itself, such as `date`
and `content-length`.

### `fixture:second-volume`

The runner creates for the case a temporary directory below the directory
given with `--second-volume` or `NURT_CONFORMANCE_SECOND_VOLUME`, writes
`second_volume_before` into it and removes it after the case.
`${second_volume}` is its path. The given directory must be on another file
system than the temporary directory of the system, which holds `work`: the
runner compares their mount points and ends with exit code 2 when they are
equal, so that a move between `work` and `${second_volume}` crosses a file
system boundary. Without the directory the cases that require the fixture are
skipped with the reason `requires fixture:second-volume`.

A continuous integration job that runs these cases provides the second file
system itself, for example on Linux a `tmpfs`:

```
mkdir -p /mnt/nurt-second
mount -t tmpfs -o size=64m tmpfs /mnt/nurt-second
nurt-conformance run --impl "nurt serve --stdio" --second-volume /mnt/nurt-second
```

On Windows a second drive or a mounted virtual disk, and on macOS an attached
disk image, serve the same purpose.

## File system snapshot `nurt-fs-snapshot/1`

A snapshot describes a directory tree. The same format is used for the initial
and the expected states of conformance cases, benchmark tasks and examples of
the documentation. Schema:
[`schema/fs-snapshot-1.schema.json`](schema/fs-snapshot-1.schema.json).

```json
{
  "format": "nurt-fs-snapshot/1",
  "entries": [
    {"path": "src/main.go", "type": "file", "content_utf8": "package main\n", "mode": "0644"},
    {"path": "assets/logo.bin", "type": "file", "content_base64": "iVBORw0KGgo="},
    {"path": "empty", "type": "dir"},
    {"path": "current", "type": "symlink", "target": "src"}
  ]
}
```

| Member | Type | Meaning |
|---|---|---|
| `format` | text | `"nurt-fs-snapshot/1"`; may be omitted when the snapshot is embedded in a case |
| `exhaustive` | boolean, default `true` | `true`: the snapshot describes the whole tree; `false`: only the listed entries are checked |
| `entries[].path` | text | path relative to the root, segments separated by `/`, without `.`, `..` and empty segments; unique |
| `entries[].type` | text | `file`, `dir` or `symlink` |
| `entries[].content_utf8` | text | content of a file as UTF-8 text |
| `entries[].content_base64` | text | content of a file in standard Base64 with padding; exactly one of the two content members is present for a file |
| `entries[].mode` | text | permission bits in octal, for example `"0755"`; applied and compared only on Unix |
| `entries[].mtime` | text | modification time of a file, RFC 3339 in UTC with whole seconds |
| `entries[].target` | text | target of a symbolic link, written as stored in the link |

Rules:

* Parent directories of an entry exist implicitly and need not be listed. A
  captured snapshot lists files, symbolic links and empty directories only,
  sorted by path in byte order.
* When an expected snapshot is compared with a directory, an entry is missing
  when it does not exist, different when its type, content, target, or (when
  given in the expected entry) mode or modification time differ, and extra
  when it exists in an exhaustive comparison without being listed or implied.
* Modes are ignored on Windows. A case that depends on symbolic links lists
  `os:unix` in `requires`.

## Binding cases `nurt-binding-cases/1`

The files `bindings/*.json` describe calls of the application interface that
every language binding offers. They are neutral to the language: each binding
has a runner that maps the calls to its idiomatic names and runs the cases on
each of its backends. Schema:
[`schema/binding-cases-1.schema.json`](schema/binding-cases-1.schema.json).

```json
{
  "format": "nurt-binding-cases/1",
  "cases": [
    {
      "id": "run.syntax-error",
      "summary": "a syntax error is returned as a response, not raised",
      "class": "mandatory",
      "steps": [
        {
          "call": "run",
          "args": {"program": "find \"/home/user\" glob: [\"*.go\""},
          "expect": {"outcome": "language_error", "code": "E_SYNTAX",
                     "fields": {"/ok": false, "/report/status": "failed"}}
        }
      ]
    }
  ]
}
```

| Member | Type | Meaning |
|---|---|---|
| `id`, `summary`, `class`, `requires` | | as in conformance cases |
| `backends` | list of text | `process` and/or `library`; all backends when absent |
| `fs` | list of snapshot entries | files with `content_utf8` and directories, created in a temporary directory before the first step; `${base}` names it, and calls that read them pass `options.base` as `"${base}"` |
| `steps[].call` | text | `run`, `start`, `check`, `parse`, `print`, `correct`, `page`, `describe`, `help`, `catalog`, `schema`, `history`, `space`, `info`, `close`; on an execution: `wait`, `cancel`, `events` |
| `steps[].execution` | integer | for `wait`, `cancel` and `events`: number of the step that called `start` |
| `steps[].args` | object | named arguments in snake case; `options` holds the request options; `raise_on_error` turns a language error into an exception |
| `steps[].expect.outcome` | text | `ok`, `language_error` or `call_error` |
| `steps[].expect.status` | text | name of the status of a call error, for example `NURT_E_ARG` |
| `steps[].expect.code` | text | error code of a language error |
| `steps[].expect.fields` | object | JSON pointer to expected value; values must be equal |
| `steps[].expect.present` | list of text | JSON pointers that must exist |
| `steps[].expect.absent` | list of text | JSON pointers that must not exist |

The pointers address the result view of the call:

| Calls | Result view |
|---|---|
| `run`, `page`, `wait` | `{"ok", "report", "text", "error", "next_cursor"}`: `ok` is `true` when the report has no error; `text` is the rendered text, when requested |
| `print`, `describe`, `help` | `{"value": text}` |
| `check`, `parse`, `correct`, `catalog`, `schema`, `history`, `space`, `info` | `{"value": object}` |
| `events` | `{"value": [events up to and including exec.finished]}` |
| `start`, `cancel`, `close` | `{}` |

A language error is `outcome: "language_error"`: the call returns a response
whose `ok` is `false`, or raises the language error exception when the step
passes `raise_on_error: true`. A call error (a status other than `NURT_OK`, a
JSON-RPC error, a failure of the server process) is `outcome: "call_error"`.
The cases never assert the value of a volatile member; they check its presence
with `present`. Placeholders `${base}` and `${stepN/pointer}` are replaced as
in conformance cases, against the result views.

## Fake server

```
nurt-conformance fake-serve --cases <directory> [--break <kind>]
nurt-conformance serve --stdio
```

The fake server speaks `nurt-rpc/1` and answers from the cases of the
directory: a request is matched to a step of a case by its method and its
parameters, and the answer is built from the expectation of that step. It
serves the tests of the runner and of the language bindings while no
implementation is available. The form `serve --stdio` reads the directory from
the environment variable `NURT_CONFORMANCE_CASES` and the kind of deviation
from `NURT_CONFORMANCE_BREAK`, so the program can replace `nurt` in a process
backend unchanged. The runner sets `NURT_CONFORMANCE_CASE` to the identifier
of the case of each server process; the fake server then parses only the case
files that contain the identifier and answers only that case or the binding
case with that identifier. Without it, the fake server loads the whole
directory, and a request that matches calls of several cases is answered from
the first of them in identifier order.

For a case that requires fixtures the runner also sets
`NURT_CONFORMANCE_CASE_HTTP_BASE` to the address of the HTTP server and
`NURT_CONFORMANCE_CASE_SECOND_VOLUME` to the directory on the second volume.
The fake server rewrites these values to `${http_base}` and `${second_volume}`
before it matches a request, and creates the expected state of the second
volume; implementations ignore the variables. The fake server sends the
`events` of a step before its response, completed with `seq`, `time` and
`request_id`, and leaves out of its answers the members named by `absent`. It
answers `testkit.clock.advance` from its own clock, which starts at
2026-01-01T00:00:00Z and gives the `time` of its events.

`--break` makes the server deviate in one way, to show that the runner detects
it:

| Kind | Deviation |
|---|---|
| `tree` | `parse` returns a tree with a changed command name |
| `bound` | `check` returns a bound tree with a changed default |
| `hash` | `check` returns a wrong `ast_hash` |
| `error-code` | language errors carry `E_INTERNAL` instead of the expected code |
| `report` | `run` reports `status: "ok"` where another status is expected |
| `schema` | `schema` returns a document that is not a valid JSON Schema |
| `effect` | `check` and `run` create the file `.nurt-conformance-effect` in `base` |
| `protocol` | every result is sent with a wrong `id` |
| `crash` | the process exits after `initialize` |
| `hang` | the server stops answering after `initialize` |
| `events` | the last expected event of every call is not sent |
| `absent` | results carry the members that `absent` names |
| `no-clock` | `testkit.clock.advance` is answered with `-32601`; the cases that call it are reported as skipped, not failed |

## Conformance report `nurt-conformance-report/1`

Schema:
[`schema/conformance-report-1.schema.json`](schema/conformance-report-1.schema.json).
The report holds the version of the suite and the digest of its cases, the
result of `info` and `initialize` of the implementation, the SHA-256 digest of
the executable that was started, the system, the result of every case (`pass`,
`fail`, `skip`, with the failures) and the counts per class and per kind. The
member `conformant` is `true` when every applicable mandatory case passed.

## Versions of the suite

The suite has its own version, `MAJOR.MINOR.PATCH` according to Semantic
Versioning 2.0.0, independent of the version of `nurt`. A patch version
corrects cases without changing what they require. A minor version adds
optional cases or mandatory cases for a new minor version of the language. A
major version changes or removes mandatory cases. The report records the
version and the digest of the case files, so a result names exactly the cases
it was obtained with.

## Submitting a result

1. Run the released version of the suite on a release build of the
   implementation, on each supported system, with `--report`.
2. Keep the reports unchanged: the digest of the cases and of the executable
   identify the tested build.
3. Send the reports, the name and version of the implementation and the
   systems tested to `support@danaco-group.pl`.

A report names the implementation conformant only for a complete run: a run
with `--filter` or `--kind` records the filters and never sets `conformant`.

The terms of use of the name and mark "Danaco Nurt Compatible" are published
separately.
