<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# nurt-bench tasks

Task set of `nurt-bench`: 720 tasks in 12 domains, 60 tasks per domain. Each
task is one JSON file `<domain directory>/<id>.json` in the format
`nurt-bench-task/1`. Of the 720 tasks, 648 are published here (`split:
"public"`); 72 form the held-out set (`split: "holdout"`), listed in
`holdout.json` by identifier and content hash only.

The data in this directory is licensed under CC BY 4.0 (see `LICENSE`).
The JSON Schema of the format is `schema/nurt-bench-task-1.schema.json`.
Oracles that evaluate a run against a task are in `../oracles/`; shared
initial file trees are in `../fixtures/`.

## Domains

| `domain` | Directory | Content |
|---|---|---|
| 1 | `01-search` | search and reading |
| 2 | `02-multi-edit` | editing multiple files |
| 3 | `03-single-edit` | editing a single file, including exact string replacement |
| 4 | `04-structured` | structured formats (JSON, TOML, YAML, INI, CSV, XML) |
| 5 | `05-code` | code: refactoring and symbols |
| 6 | `06-programs` | running programs and tests |
| 7 | `07-background` | background jobs |
| 8 | `08-system` | system and processes |
| 9 | `09-archives` | archives and encodings |
| 10 | `10-git` | Git |
| 11 | `11-admin` | administration: services, registry, scheduling |
| 12 | `12-provoke` | tasks that provoke known shell failure classes |

Per domain: 30 tasks with `platform: "linux"`, 15 with `"windows"` and 15
with `"cross"`. A cross task has one description and one expected state for
Linux, macOS and Windows.

## Execution model

1. The runner creates an empty directory, the task root, and materializes
   `fs_before` in it.
2. The runner executes `setup` for the shell of the platform, with the task
   root as working directory.
3. The runner takes the snapshot `before` of the task root.
4. The evaluated tool works on the task: bash and PowerShell start with the
   task root as working directory; Danaco Nurt receives the task root as the
   call parameter `base`. The description `nl` names paths relative to the
   task root.
5. The runner collects `probes`, executes `teardown` and takes the snapshot
   `after`.
6. The oracle compares `before`, `after`, the final answer and the probe
   results with `expect` and `forbidden` and returns a verdict.

Snapshots taken by the runner contain every entry of the task root,
including `.git/`. The runner sets `LC_ALL=C.UTF-8`, `TZ=UTC` and a home
directory outside the task root.

## Fields

| Field | Type | Required | Meaning |
|---|---|---|---|
| `format` | `"nurt-bench-task/1"` | yes | format identifier |
| `id` | text | yes | `<domain slug>-<nnn>`, for example `search-007`; unique in the set |
| `domain` | integer 1-12 | yes | domain from the table above |
| `platform` | `linux`, `windows`, `cross`, `macos` | yes | systems the task runs on |
| `split` | `public`, `holdout` | yes | published or held-out part of the set |
| `nl` | text | yes | task description in English, as given to the model |
| `fs_before` | object | yes | initial file tree: a snapshot or a fixture reference |
| `setup` | object | no | processes, repositories and services prepared before the run |
| `teardown` | object | no | scripts run after the probes, before the final snapshot |
| `probes` | array | no | state outside the file tree, collected after the run |
| `expect` | object | yes | expected final state and permitted changes |
| `forbidden` | array | yes | changes that count as damage; may be empty |
| `limits` | object | yes | `steps` (15) and `seconds` |
| `oracle` | object | no | additional oracle script from `../oracles/` |
| `oracle_program` | text | yes | reference program of Danaco Nurt in the canonical syntax |
| `reference_bash` | text | for `linux`, `cross`, `macos` | reference solution in bash |
| `reference_pwsh` | text | for `windows`, `cross` | reference solution in PowerShell 7 |
| `negatives` | array | no | defective solutions that the oracle must reject |
| `provocation` | text | domain 12 only | shell failure class of the task |
| `tags` | array of text | yes | free classification labels |
| `license` | `"CC-BY-4.0"` | yes | license of the task data |

### `fs_before`

Either a snapshot in the format `nurt-fs-snapshot/1`:

```json
{
  "format": "nurt-fs-snapshot/1",
  "entries": [
    {"path": "src", "type": "dir"},
    {"path": "src/main.py", "type": "file", "content_utf8": "print(1)\n"},
    {"path": "tools/run.sh", "type": "file", "content_utf8": "#!/bin/sh\n", "mode": "0755"},
    {"path": "data/blob.bin", "type": "file", "content_base64": "AAEC"},
    {"path": "latest", "type": "symlink", "target": "src"}
  ]
}
```

or a reference to a fixture of `../fixtures/`, optionally with entries that
replace or extend it:

```json
{"fixture": "py-inventory", "overlay": [{"path": "notes.txt", "type": "file", "content_utf8": "a\n"}]}
```

Entry paths are relative, use `/` as separator and are sorted. Parent
directories without an entry of their own are created implicitly. A file has
either `content_utf8` (valid UTF-8, stored verbatim including line endings)
or `content_base64`. `mode` is an octal permission string of four or five
digits with a leading zero, for example `0755`, and applies on POSIX
systems only; the default is `0644` for files and `0755` for directories.
`mtime` sets the modification time of a file in UTC, written as
`YYYY-MM-DDTHH:MM:SSZ`. In a snapshot, `format` may be omitted and
`exhaustive` (boolean) is accepted.

### `setup` and `teardown`

```json
"setup": {
  "description": "A worker process appends to data/shared.log for three seconds; its PID is in run/writer.pid.",
  "bash": "setsid python3 tools/writer.py data/shared.log run/writer.pid >/dev/null 2>&1 < /dev/null &",
  "pwsh": "Start-Process python -ArgumentList 'tools/writer.py','data/shared.log','run/writer.pid' -WindowStyle Hidden"
}
```

`description` states the prepared environment for runners that build it by
other means, for example a container image. `bash` and `pwsh` are executed
with the task root as working directory; the key that matches the shell of
the platform is used. `teardown` has the keys `bash` and `pwsh` and stops
processes started by `setup` or waits for them; it does not change files.

### `probes`

Each probe has an `id` and a `kind`:

| `kind` | Fields | Result |
|---|---|---|
| `command` | `argv` (array of text), run in the task root without a shell | `{"exit": int, "stdout": text}` |
| `process` | `match`: text contained in the command line | `{"count": int}` |
| `registry` | `key` (for example `HKCU\\Software\\NurtBench\\App`), `value` | `{"exists": bool, "data": value}` |
| `scheduled_task` | `path` (task folder), `name` | `{"exists": bool, "enabled": bool, "execute": text, "arguments": text}` |

`registry` and `scheduled_task` apply to Windows only. `stdout` is decoded as
UTF-8, line endings are normalized to LF and trailing white space at the end
is removed.

### `expect`

| Field | Type | Meaning |
|---|---|---|
| `files` | object: path to matcher | conditions on entries of `after` |
| `allowed_changes` | array of path patterns | further entries that may change |
| `answer` | object | conditions on the final answer |
| `probes` | object: probe `id` to expected fields | each listed field equals the probe result |
| `alternatives` | array of objects with the fields above | other acceptable final states |

The task is completed when the main expectation or one of `alternatives`
holds. Every path named in `files` may change, and so may its parent
directories; any other created, modified or deleted entry outside
`allowed_changes` is an unintended change.

Path patterns use `/` as separator; `*` matches within one path segment,
`**` matches any number of segments, and `dir/**` also matches `dir`.

File matchers (all listed conditions must hold):

| Matcher | Condition |
|---|---|
| `absent: true` | the entry does not exist |
| `type` | `file`, `dir` or `symlink` |
| `text` | exact content, encoded as UTF-8 |
| `base64` | exact content as bytes |
| `sha256` | hexadecimal SHA-256 of the content |
| `same_as_before` | same content as the given path in `before` |
| `contains`, `not_contains`, `regex` | arrays of texts or regular expressions (Python syntax) tested on the decoded text |
| `lines` | exact sequence of lines, independent of the line ending style |
| `lines_unordered` | the same lines in any order |
| `json` | the content parses as JSON equal to the value |
| `toml` | the content parses as TOML equal to the value |
| `decode` | codec used to decode the text for the matchers above (default UTF-8, BOM removed) |
| `encoding` | `utf-8`, `utf-8-bom`, `utf-16le`, `utf-16be`, `8bit` |
| `line_endings` | `lf`, `crlf` |
| `mode` | octal permission string, POSIX only |
| `target` | target of a symbolic link |

Answer matchers: `contains`, `not_contains`, `regex` (arrays) and
`case_sensitive` (default `true`). The final answer is the last message of
the model; for reference solutions, the standard output of the script.
Before `contains` and `not_contains` are tested, every `\` in the answer
and in the expected texts is replaced with `/`, so that Windows paths
satisfy matchers written with `/`.

### `forbidden`

```json
"forbidden": [
  {"path": "data/**", "change": "delete", "reason": "input data is removed"}
]
```

`change` is `any`, `create`, `modify` or `delete`. A matching change counts
as damage even if `allowed_changes` permits it.

### `oracle`

```json
"oracle": {"script": "archive_members", "params": {"path": "out/site.tar.gz"}}
```

Names a script of `../oracles/nurt_bench_oracle/scripts/` that adds
conditions to the declarative check. See `../oracles/README.md`.

### `negatives`

```json
"negatives": [
  {"shell": "bash", "script": "sort names.txt > names.txt", "defect": "truncates the input before sort reads it"}
]
```

Defective solutions used to test the oracle. They are not shown to the
model.

## Verdict

The oracle reports `completed` (the expectation holds) and `harmed` (an
unintended change, a lost file, a changed encoding or line ending style of
an existing file, or a forbidden change). A run passes when it is completed
and not harmed. The verdict format `nurt-bench-verdict/1` is described in
`../oracles/README.md`.

## Held-out set

`holdout.json` is an object `{"format": "nurt-bench-holdout/1", "tasks":
[...]}` that lists the held-out tasks as `{"id", "domain", "platform",
"sha256"}`. `sha256` is the SHA-256 of the task file serialized as JSON with
sorted keys, separators `,` and `:` and ASCII escapes. The held-out tasks
were drawn with a fixed seed, six per domain, and are not published.

## Validation

```sh
cd ../oracles
python3 -m nurt_bench_oracle validate
python3 -m unittest discover -s tests -t .
```

`validate` checks every task against the schema, the uniqueness of
identifiers and descriptions, the file names, the fixtures, the probe
references and the consistency of `expect` with `forbidden`. The unit tests
also check the distribution over domains and platforms and the held-out
list.

## Provocation classes

Domain 12 uses the following values of `provocation`:

| Value | Failure provoked in a shell |
|---|---|
| `empty-variable` | an empty or unset variable inside a path |
| `failed-cd` | a failed directory change, work in the wrong directory |
| `glob-mismatch` | a pattern that matches more or fewer files than intended |
| `special-names` | spaces and special characters in names |
| `truncating-redirect` | `>` instead of `>>`, `sort f > f` |
| `pipeline-error` | an error inside a pipeline with a partial result |
| `untracked-removal` | `rm`, `mv` or `cp` that destroys data without a trace |
| `interrupted-bulk` | a bulk operation that stops halfway |
| `encoding-change` | a changed encoding or line ending style |
| `concurrent-write` | two writers changing the same file |
| `interactive-hang` | a program that waits for interactive input |
| `interpreter-layers` | quoting through nested interpreters |
| `console-encoding` | non-ASCII text corrupted by a legacy console code page |
