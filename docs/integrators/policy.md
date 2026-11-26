<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Policy

The console runs the programs it receives; it does not decide which programs an agent may run. The host enforces its policy before `run`, on data that the console computes without any effect:

| Call | Effect | Returns |
|---|---|---|
| `check` (`nurt serve`), `nurt_check` (C ABI) | none; parse, correction, binding and plan | the strongest effect, the weakest reversibility, the paths named by the program, the network targets, the programs to run, the command groups, the corrections |
| `run` with `preview: true` | none; stages up to the plan of the effect | the targets the program would change, with differences, `notable` and `irreversible` effects known in advance |

The answer of `check` is the document `nurt-check/1`:

| Field | Content |
|---|---|
| `format` | `nurt-check/1` |
| `effect` | strongest effect of the plan: `read`, `local` (changes local files), `external` (acts outside the local file system, for example `git.push` or a `POST` request) or `process` (starts programs) |
| `reversibility` | weakest reversibility: `read`, `reversible`, `reversible_if_preserved` or `irreversible` |
| `operations` | number of operations; more than one for a script |
| `waits` | `true` when a stage blocks the call by waiting |
| `paths` | paths named by literals of the program, in order of appearance |
| `network` | one entry per endpoint: `command`, `param`, `source` (`literal`, `record`, `remote`, `catalog`, `implementation`), `target` (`scheme`, `host`, `port`, `url`; `null` when unknown before the run) and `remote` |
| `programs` | one entry per external program: `command`, `program`, `implementation`, `launcher`, `source` |
| `groups` | command groups the plan uses, in order of first use |
| `corrections` | corrections applied to the program text before binding: `id`, `rule`, `class`, `at`, `from`, `to`, `kind`, `alternatives` |
| `details` | facts reported by the static check of the commands of the program: `command`, `index`, `details` |

The method `check` of `nurt serve` returns this document with the member `diagnostics` (the diagnostics of parsing and binding); both parts are required in the result. `nurt_check` of the C ABI returns the document `nurt-check/1`.

The examples use the Python binding, where `CheckResult.raw` holds the document.

Policy decisions are made on the corrected and bound program and on the actual targets, never on the program text.

An example of the document for a program that pushes a branch:

<!-- schema: check -->
```json
{
  "format": "nurt-check/1",
  "effect": "external",
  "reversibility": "irreversible",
  "operations": 1,
  "waits": false,
  "paths": ["/home/user/app"],
  "network": [
    {"command": "git.push", "param": null, "source": "remote", "target": null, "remote": "origin"}
  ],
  "programs": [],
  "groups": ["git"],
  "corrections": [],
  "details": []
}
```

The reply of `nurt serve` to `check` for the program above:

<!-- schema: rpc -->
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "format": "nurt-check/1",
    "effect": "external",
    "reversibility": "irreversible",
    "operations": 1,
    "waits": false,
    "paths": ["/home/user/app"],
    "network": [
      {"command": "git.push", "param": null, "source": "remote", "target": null, "remote": "origin"}
    ],
    "programs": [],
    "groups": ["git"],
    "corrections": [],
    "details": [],
    "diagnostics": []
  }
}
```

## Forbidding a command: `git.push`

```python
check = nurt.check(program)
commands = {use["command"] for use in check.raw["network"]}
commands |= {facts["command"] for facts in check.raw["details"]}
if "git.push" in commands:
    return tool_error("git.push is not allowed in this session")
response = nurt.run(program)
```

`check` runs after correction and binding and names every command with its full name, so an alias, a different notation or a corrected spelling of a command that only reads is caught. A misspelt name of a command that changes something is not corrected: `check` answers `E_AMBIGUOUS` and the first candidate is the program with the name written right, so a forbidden command cannot be reached by a typing error either. A host that allows no effect outside the local file system compares `effect` with `external` instead.

## Allowed directories

```python
ALLOWED = ("/home/user/app/",)

check = nurt.check(program, base="/home/user/app")
outside = [path for path in check.raw["paths"] if not path.startswith(ALLOWED)]
if outside:
    return tool_error(f"paths outside the workspace: {outside}")
preview = nurt.run(program, base="/home/user/app", preview=True)
targets = [change["path"] for change in preview.report.get("changes", [])]
if any(not path.startswith(ALLOWED) for path in targets):
    return tool_error("the program changes files outside the workspace")
```

`check` lists the paths written in the program; `preview` lists the files that the program would change (`report.changes`) after glob expansion and filtering. Both are needed: a pattern inside an allowed directory can still reach a symbolic link that leads outside.

## Network targets of `http.*`

For the commands that use the network, `check` returns every endpoint the program contacts: the command, the origin of the target and, when it is known before the run, the scheme, host, port and URL. A host that allows access to one service:

```python
check = nurt.check(program)
for use in check.raw["network"]:
    target = use["target"]
    if target is None or target["host"] != "api.example.com" or target["scheme"] != "https":
        return tool_error(f"network target not allowed: {use['command']}")
```

Targets computed at run time (a URL read from a file) have `source: "record"` and `target: null`; a strict policy rejects such programs or runs them with `preview: true` first. A host that must allow reads only compares `effect` of the document with `read`.

## Programs started by `run`

`programs` of the document lists the external programs that the plan would start, with the command that starts each one and the origin of the program name. The policy of a host for shell commands applies to this list. Programs known to change remote state (publishing, deployment, remote shells) are reported in `irreversible` with the reason `external_effect`.

## Confirmation by the user

A host that asks the user before a change runs the program with `preview: true`, shows the targets and differences of the report, and then runs the same program without `preview`. The journal records every change; `undo` reverts an operation by its number `op`.
