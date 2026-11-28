<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/reference/serve-protocol.md -->
<!-- Responsibility:   Documents `nurt serve` protocol reference (`nurt-rpc/1`). -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# `nurt serve` protocol reference (`nurt-rpc/1`)

`nurt serve` runs the engine behind a JSON-RPC 2.0 server on standard input
and standard output. Embedding applications that cannot load the C library
(`docs/reference/c-abi.md`) use this protocol; both interfaces return the same
JSON documents and map failures to the same status codes.

## Synopsis

```
nurt serve [--framing ndjson|content-length]
```

| Option | Type | Default | Description |
|---|---|---|---|
| `--framing` | `ndjson` \| `content-length` | `ndjson` | Delimiting of messages on both streams. |

Standard output carries protocol messages only. Diagnostics of the process
go to standard error or, after `initialize` with events enabled, to `log`
events.

## Framing

| Framing | Form of one message |
|---|---|
| `ndjson` | One JSON document per line, UTF-8, terminated by `\n`. A `\r` before the `\n` and empty lines are ignored. The server never writes a line feed inside a message. |
| `content-length` | Header lines terminated by `\r\n`, an empty line, then exactly `Content-Length` bytes of JSON. Other headers are ignored. The server writes only the `Content-Length` header. |

A received message longer than the size limit (16 MiB unless the server was
configured otherwise; reported in `initialize` as `limits.max_message_bytes`)
is skipped and answered with `-32600` and `id: null`. A broken header block of
the `content-length` framing is answered with `-32700` and ends the
connection with exit code 1.

## Messages

Every message is a JSON-RPC 2.0 object with `"jsonrpc": "2.0"`.

| Message | Members | Server behaviour |
|---|---|---|
| Request | `id` (string, number or `null`), `method`, `params` (object, array or absent) | Answered with exactly one reply carrying the same `id`. |
| Notification | `method`, `params`, no `id` | Executed; never answered, also on error. |
| Reply sent by the client | `id`, `result` or `error` | Ignored. The server sends no requests. |
| Batch (array) | | Rejected with `-32600`; send one request per message. |

## Connection lifecycle

1. The client sends `initialize`. Before it, every method except
   `initialize` and `shutdown` is answered with `-32600`.
2. The client sends requests. Several requests may be in flight; replies
   arrive in the order the requests complete and are matched by `id`.
3. The client sends `shutdown`, or closes the input. The server stops
   reading, waits until every request in flight has been answered, answers
   `shutdown` with `null` and exits. Messages after `shutdown` are not read.

| Exit code | Meaning |
|---|---|
| `0` | `shutdown` was received or the input ended. |
| `1` | The input could not be read, the framing was broken, or the output could not be written. |

When a write to the output fails, the server cancels the requests in flight and
reads no further message: it exits with code 1 before the next read, or, when a
read is already waiting, as soon as that read returns; the message it returned
is not processed. The server never waits for the end of the input.

## Methods

| Method | Parameters | Result |
|---|---|---|
| `initialize` | see [`initialize`](#initialize) | see [`initialize`](#initialize) |
| `run` | request envelope `nurt-request/1` | response `nurt-response/1`; a request whose only fault is an `ast` that is not a tree `nurt-ast/1` is answered with the language error `E_AST_SCHEMA` (report status `failed`), not with `-32602` |
| `check` | request envelope `nurt-request/1` | check document `nurt-check/1` with the members `bound` (the bound tree), `ast_hash` (hash of the bound tree in `bound`, as in `spec/syntax-tree.md` section 6, `sha256:<hex>`; absent only when the tree is not known, never for a program given as text) and `diagnostics` |
| `parse` | `{program: string, variant?: "a" \| "b" \| "c"}` | `{ast: nurt-ast/1, diagnostics: []}` |
| `print` | `{ast: nurt-ast/1, variant?: "a" \| "b" \| "c"}` | `{text: string}` |
| `correct` | `{program: string, variant?: "a" \| "b" \| "c"}` | `{ast: nurt-ast/1, corrections?: [...], ran?: string}`; `ran` is the corrected program text when the engine corrected it |
| `page` | `{cursor: string, limit?: integer}` | response `nurt-response/1` with the next page |
| `describe` | `{subject?: string, language?: string}` | `{description: {text: string, tokens: integer, estimate: boolean}}`; the description of the subject is the field `text` |
| `help` | `{topic?: string, language?: string}` | `{text: string}` |
| `schema` | `{name: string}` | the published JSON Schema `name` |
| `catalog` | `{what?: object \| null}` or absent | command catalog `nurt-catalog-result/1` |
| `history` | `{query?: object \| string \| null}` or absent | `{operations: [...]}` |
| `space` | `{volume?: absolute path}` | `{volumes: [...]}` |
| `info` | absent | engine description: version, formats, features, system |
| `cancel` | `{request_id}` or `{job_id}` | `{cancelled: boolean}` |
| `shutdown` | absent | `null` |

`variant` defaults to the variant of the configuration; `subject` and
`topic` default to `core`. `schema` accepts `ast`, `bound`, `report`, `request`,
`response`, `config`, `catalog`, `catalog-result`, `value`, `tool`,
`fs-snapshot`, `check`, `rpc` and `event`. `what` selects
`{"group": name}` (the commands of a group) or `{"command": name}` (the
specification of a command); `null` or absent returns the groups. `query`
is the query of `history` as an object or as the text of an object; an
object without `query` is itself the query. A selection or a query of another
shape is the JSON-RPC error `-32602`. A language error of any
method is part of the result, not a JSON-RPC error: the result is then a
response `nurt-response/1` whose report has the status `failed`, the error in
`report.error` and, as `report.kind`, the name of the method (for `run`, the
kind of the program).

### `initialize`

| Parameter | Type | Required | Description |
|---|---|---|---|
| `protocol` | string | yes | `nurt-rpc/<major>[.<minor>]`; the major version must be `1`. |
| `client` | object `{name, version}` | no | Name and version of the client; informational. |
| `abi` | integer | no | Expected C interface version `(major << 16) \| minor`; compared by major version when the server embeds the C interface. |
| `language` | string | no | Expected language version; informational. |
| `ast` | string | no | Expected version of `nurt-ast/1`; the major version must match. |
| `session` | string | no | Session used by `run`, `check` and `page` requests whose `options.session` is absent. |
| `config_overrides` | object | no | Must be absent or empty; the configuration is fixed when the server starts. |
| `capabilities.events` | boolean | no, `false` | Send `nurt/event` notifications for executions. |

Unknown members are ignored.

| Result member | Type | Description |
|---|---|---|
| `protocol` | string | `nurt-rpc/1`. |
| `version` | string | Version of Danaco Nurt. |
| `abi` | integer \| `null` | Version of the embedded C interface; `null` when the server does not embed it. |
| `language` | `null` | The engine does not report a language version. |
| `ast` | string | Version of `nurt-ast/1`. |
| `report` | string | Version of `nurt-report/1`. |
| `catalog_rev` | `null` | The engine does not report a catalog revision. |
| `features` | array of string | Features of the build, for example `stub`. |
| `groups` | array of string | Command groups whose services the engine provides. |
| `limits.max_inflight` | integer | Requests executed at the same time (default 16). |
| `limits.max_message_bytes` | integer | Size limit of one received message (default 16777216). |

Errors: `-32602` for invalid parameters or non-empty `config_overrides`,
`-32004` (`NURT_E_ABI`) for another major version of `protocol`, `abi` or
`ast` (`data.supported` names the supported version), `-32600` when the
connection is already initialized.

### `cancel`

Exactly one of the members is given.

| Parameter | Type | Description |
|---|---|---|
| `request_id` | string \| number | `id` of a request in flight on this connection. The comparison keeps the JSON type: `1` and `"1"` differ. |
| `job_id` | string | Identifier of a background job. Not available in this server: the result is a `failed` response with `E_UNSUPPORTED`. |

`cancelled` is `true` when the request was in flight. Cancelling is
idempotent; a finished or unknown request gives `false` and no error. A
cancelled `run` is answered with `-32002` (`NURT_E_CANCELLED`) whose
`data.response` is the `failed` response with `E_CANCELLED`, and its last
event is `exec.finished` with `error: "E_CANCELLED"`. Methods other than `run`
complete normally.

## Errors

The `error` object of a reply is used only for errors of the protocol and of
the call. Language errors are part of the result.

| Code | Name | Cause |
|---|---|---|
| `-32700` | Parse error | The message is not JSON, or the framing is broken. |
| `-32600` | Invalid request | The message is not a request object; a batch; the message is too long; the connection is not initialized or already initialized; the limit of requests in flight is reached; a request with the same `id` is in flight. |
| `-32601` | Method not found | The method is unknown; the message lists the known methods. |
| `-32602` | Invalid params | The parameters do not match the method. |
| `-32603` | Internal error | The server failed to serialise a result. |
| `-32000 - s` | Application error | Status `s` of the C interface; `data.status` names it. |

| Code | `nurt_status` | Cause in the server |
|---|---|---|
| `-32001` | `NURT_E_TIMEOUT` | Not used by the server. |
| `-32002` | `NURT_E_CANCELLED` | A `run` was cancelled; `data.response` holds the response. |
| `-32003` | `NURT_E_ARG` | A recorded stub case fails with this status. |
| `-32004` | `NURT_E_ABI` | Version mismatch in `initialize`. |
| `-32005` | `NURT_E_IO` | A recorded stub case fails with this status. |
| `-32006` | `NURT_E_INTERNAL` | The execution of a request panicked (`data.panic`), or no stub case answers the call. |

## Concurrency

Each request runs on its own thread; `initialize`, `cancel` and `shutdown`
are answered by the reading thread, so a cancellation is never queued behind
the request it cancels. At most `limits.max_inflight` requests and
notifications run at the same time; a request above the limit is answered
with `-32600`. An `id` may be reused once the reply to the previous request
with that `id` was sent.

## Events

After `initialize` with `capabilities.events = true`, each execution of `run`
sends notifications with the method `nurt/event`. The parameters are the
delivered event of the contract:

| Member | Type | Description |
|---|---|---|
| `seq` | integer | Number of the event within the execution, from 1. |
| `time` | string | Time of emission, RFC 3339 UTC. |
| `request` | string \| `null` | `id` of the request as text; `null` for a notification. |
| `t` | string | Event type. |

| `t` | Further members |
|---|---|
| `exec.started` | `ast_hash`, `effect` (`effect` is absent when the engine does not report it) |
| `progress` | `command`, `phase`, `done`, `total`, `bytes`, `message` |
| `job.started` | `id`, `label`, `command`, `cwd`, `pid`, `started` |
| `job.output` | `id`, `stream`, `cursor`, `lines`, `bytes` |
| `job.progress` | `id`, `percent`, `line` |
| `job.finished` | `id`, `state`, `exit_code`, `signal`, `elapsed`, `output_file`, `changes` |
| `log` | `level`, `message`, `source` |
| `exec.finished` | `status`, `error` (the code of the error of the report, `null` when the report has none; also for a program that failed before it was planned, for example `E_SYNTAX`) |

The events of one execution are sent in the order of `seq`; `exec.started`
is the first and `exec.finished` the last, and all of them precede the reply
to the request.

## Stub mode

A build with the feature `stub` can answer from the recorded cases of the
C library stub (`crates/nurt-ffi/tests/fixtures/stub/`) instead of an engine,
for tests of bindings before an engine is available. A call is answered by
the case whose key, the SHA-256 digest of the canonical JSON of
`{"function", "input"}`, equals the key of the call; the input of a call is
built as in the C interface. `initialize` and `info` report the feature
`stub`. A call without a case is answered with `-32006` and `data.key`.

## Examples

Newline-delimited session (`>` sent by the client, `<` sent by the server):

```
> {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol":"nurt-rpc/1","capabilities":{"events":true}}}
< {"jsonrpc":"2.0","id":1,"result":{"protocol":"nurt-rpc/1","version":"0.1.2","abi":null,"language":null,"ast":"1.1","report":"1.0","catalog_rev":null,"features":[],"groups":[],"limits":{"max_inflight":16,"max_message_bytes":16777216}}}
> {"jsonrpc":"2.0","id":2,"method":"run","params":{"nurt":"1.0","program":"find \"/srv/app\" glob: \"*.go\""}}
< {"jsonrpc":"2.0","method":"nurt/event","params":{"seq":1,"time":"2026-11-21T09:12:00Z","request":"2","t":"exec.started"}}
< {"jsonrpc":"2.0","method":"nurt/event","params":{"seq":2,"time":"2026-11-21T09:12:00Z","request":"2","t":"exec.finished","status":"ok","error":null}}
< {"jsonrpc":"2.0","id":2,"result":{"nurt":"1.0","report":{"nurt":"1.0","status":"ok","kind":"find"}}}
> {"jsonrpc":"2.0","id":3,"method":"shutdown"}
< {"jsonrpc":"2.0","id":3,"result":null}
```

Cancelling a request in flight:

```
> {"jsonrpc":"2.0","id":"scan","method":"run","params":{"nurt":"1.0","program":"find \"/srv/archive\""}}
> {"jsonrpc":"2.0","id":5,"method":"cancel","params":{"request_id":"scan"}}
< {"jsonrpc":"2.0","id":5,"result":{"cancelled":true}}
< {"jsonrpc":"2.0","id":"scan","error":{"code":-32002,"message":"the request was cancelled","data":{"status":"NURT_E_CANCELLED","response":{"nurt":"1.0","report":{"nurt":"1.0","status":"failed","kind":"find","error":{"code":"E_CANCELLED","message":"the execution was cancelled"}}}}}}
```

The same request with the `content-length` framing:

```
Content-Length: 44\r\n
\r\n
{"jsonrpc":"2.0","id":3,"method":"shutdown"}
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
