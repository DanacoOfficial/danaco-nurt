<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Remote instances and events

## Syntax trees for another device

A host that runs programs on another machine (a server, a container, a device) parses the program locally and sends the syntax tree:

1. `check` (or `parse`) on the local console returns the tree `nurt-ast/1` after correction.
2. The host applies its policy to the bound tree.
3. The host sends a request with `ast` to the remote instance over its own transport (SSH, a socket, a message queue) to `nurt serve` running there:

```json
{"nurt": "1.0", "ast": {"nurt": "1.0", "program": {"statements": []}}, "options": {"base": "/srv/app", "session": "s-17"}}
```

The remote instance does not correct the tree again, so the program that runs is the one the policy checked. A tree with a newer version than the instance supports is rejected with `E_AST_VERSION`, which lists the unknown nodes. The connection, authentication and the choice of the device belong to the host.

## Events

`nurt serve` sends notifications `nurt/event` without an `id`. Each event has `t` (type), `seq`, `time` and, where it applies, `request_id` or `job_id`.

| Type | Content |
|---|---|
| `job.started` | `id`, `label`, `command`, `cwd`, `pid`, `started` |
| `job.output` | `id`, `stream` (`stdout`, `stderr`), `cursor`, `lines`, `bytes` |
| `job.progress` | `id`, `percent`, `line` |
| `job.finished` | `id`, `state`, `exit_code`, `signal`, `elapsed`, `output_file`, `changes` |
| `journal.committed` | `op`, `kind`, `files`, `bytes` |
| `journal.expired` | `op`, `reason` |
| `space.state` | free space state of a volume |
| `schedule.fired` | scheduled task that started |

A program started with `run ... background: true` returns a `job` record at once. The host follows the job through the events or reads its output incrementally with `jobs <id> since: <cursor>`; `stop`, `retry` and `remove` act on the job. Subscriptions, filtering of output lines and delivery of events to the context of the model belong to the host.
