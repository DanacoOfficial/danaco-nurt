<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# Remote instances and events

## Syntax trees for another device

A host that runs programs on another machine (a server, a container, a device) parses the program locally and sends the syntax tree:

1. `parse` on the local console returns the tree `nurt-ast/1` after correction.
2. The host applies its policy with `check` on the same program.
3. The host sends a request with `ast` to the remote instance over its own transport (SSH, a socket, a message queue) to `nurt serve` running there:

<!-- schema: request -->
```json
{
  "nurt": "1.0",
  "ast": {
    "nurt": "1.0",
    "program": {
      "statements": [
        {
          "t": "pipeline",
          "head": {
            "t": "stage",
            "name": "list",
            "args": [{"value": {"t": "text", "v": "/srv/app"}}]
          },
          "stages": []
        }
      ]
    }
  },
  "options": {"base": "/srv/app", "session": "s-17"}
}
```

The remote instance does not correct the tree again, so the program that runs is the one the policy checked. A tree with a newer version than the instance supports is rejected with `E_AST_VERSION`, which lists the unknown nodes. The connection, authentication and the choice of the device belong to the host.

## Events

`nurt serve` sends notifications `nurt/event` without an `id`; the parameters are the document `nurt-event/1`, which `libnurt` also passes to `nurt_event_cb`. Each event has `t` (type), `seq` (from 1, within the execution), `time` and, when the request has an identifier, `request`.

| Type | Content |
|---|---|
| `exec.started` | `ast_hash`, `effect` |
| `exec.finished` | `status`, `error` |
| `progress` | `command`, `phase`, `done`, `total`, `bytes`, `message` |
| `job.started` | `id`, `label`, `command` (list of strings), `cwd`, `pid`, `started` |
| `job.output` | `id`, `stream` (`stdout`, `stderr`), `cursor`, `lines`, `bytes` |
| `job.progress` | `id`, `percent`, `line` |
| `job.finished` | `id`, `state` (`exited`, `failed`, `stopped`, `timed_out`), `exit_code`, `signal`, `elapsed`, `output_file`, `changes` |
| `log` | `level`, `message`, `source` |

A notification in the stream of `nurt serve`:

<!-- schema: rpc -->
```json
{
  "jsonrpc": "2.0",
  "method": "nurt/event",
  "params": {
    "seq": 3,
    "time": "2026-11-22T10:15:02Z",
    "request": "7",
    "t": "job.output",
    "id": "j-4",
    "stream": "stdout",
    "cursor": "o-120",
    "lines": ["compiling", "linking"],
    "bytes": 17
  }
}
```

The same event as the parameter of `nurt_event_cb`:

<!-- schema: event -->
```json
{"seq": 4, "time": "2026-11-22T10:15:09Z", "request": "7", "t": "exec.finished", "status": "ok", "error": null}
```

A program started with `run ... background: true` returns a `job` record at once. The host follows the job through the events or reads its output incrementally with `jobs <id> since: <cursor>`; `stop`, `retry` and `remove` act on the job. Subscriptions, filtering of output lines and delivery of events to the context of the model belong to the host.
