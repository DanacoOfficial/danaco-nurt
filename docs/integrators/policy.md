<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Policy

The console runs the programs it receives; it does not decide which programs an agent may run. The host enforces its policy before `run`, on data that the console computes without any effect:

| Call | Effect | Returns |
|---|---|---|
| `check` (`nurt serve`), `nurt_check` (C ABI) | none; parse, correction and binding | bound syntax tree, corrections, the effect of every stage, the paths named by the program, the programs to run with their arguments, the network targets |
| `run` with `preview: true` | none; stages up to the plan of the effect | the targets the program would change, with differences, `notable` and `irreversible` effects known in advance |

The answer of `check` is the document `nurt-check/1`:

| Field | Content |
|---|---|
| `bound` | bound syntax tree `nurt-bound/1` |
| `effects` | one entry per stage: `command` (full name) and `effect` |
| `paths` | paths named by the program, resolved against `base` |
| `external_programs` | programs that `run` and `batch` would start, resolved through `PATH`, with their arguments |
| `network` | requests the program sends: `method`, `url`, `changes_remote` |
| `waits` | waits and time limits of the program |
| `notable` | unusual effects known before the run |

`effect` is `read`, `local` (changes local files), `external` (changes state outside the machine, for example `git.push` or a `POST` request) or `process` (starts programs). The examples use the Python binding, where `CheckResult.raw` holds the document.

Policy decisions are made on the bound tree and on the actual targets, never on the program text.

## Forbidding a command: `git.push`

```python
check = nurt.check(program)
if any(stage["command"] == "git.push" for stage in check.raw["effects"]):
    return tool_error("git.push is not allowed in this session")
response = nurt.run(program)
```

The bound tree names every command with its full name, so an alias, a different notation or a corrected spelling of the command is caught.

## Allowed directories

```python
ALLOWED = ("/home/user/app/",)

check = nurt.check(program, base="/home/user/app")
outside = [path for path in check.raw["paths"] if not path.startswith(ALLOWED)]
if outside:
    return tool_error(f"paths outside the workspace: {outside}")
preview = nurt.run(program, base="/home/user/app", preview=True)
targets = [target["path"] for target in preview.report.get("targets", [])]
if any(not path.startswith(ALLOWED) for path in targets):
    return tool_error("the program changes files outside the workspace")
```

`check` lists the paths written in the program, resolved against `base`; `preview` lists the files that the program would change after glob expansion and filtering. Both are needed: a pattern inside an allowed directory can still reach a symbolic link that leads outside.

## Network targets of `http.*`

For the commands of the `network` group, `check` returns every request the program sends: the method, the URL and whether the request changes remote state. A host that allows read-only access to one service:

```python
check = nurt.check(program)
for request in check.raw["network"]:
    host = urllib.parse.urlsplit(request["url"]).hostname
    if host != "api.example.com" or request["method"] not in ("GET", "HEAD"):
        return tool_error(f"request not allowed: {request['method']} {request['url']}")
```

Targets computed at run time (a URL read from a file) are reported as unresolved; a strict policy rejects such programs or runs them with `preview: true` first.

## Programs started by `run`

`check` lists the programs that `run` and `batch` would start, resolved through `PATH`, with their arguments. The policy of a host for shell commands applies to this list. Programs known to change remote state (publishing, deployment, remote shells) are reported in `irreversible` with the reason `external_effect`.

## Confirmation by the user

A host that asks the user before a change runs the program with `preview: true`, shows the targets and differences of the report, and then runs the same program without `preview`. The journal records every change; `undo` reverts an operation by its number `op`.
