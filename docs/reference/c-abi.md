<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# C interface (libnurt)

The library `libnurt` exposes the engine to C and to every language with a C
foreign function interface. The interface is declared in
`crates/nurt-ffi/include/nurt.h`; the exported symbols are listed in
`crates/nurt-ffi/nurt.symbols`. The crate `nurt-ffi` builds a shared library
(`libnurt_ffi.so`, `libnurt_ffi.dylib`, `nurt_ffi.dll`) and a static library.
The release publishes the shared library in its own archive `nurt-lib-<version>-<target>`, as `libnurt.so.1`,
`libnurt.1.dylib` and `nurt.dll`, with `nurt.h` and `nurt.pc`; the program archive does not hold it. The packages
install the library as `libnurt.so` (also `libnurt.so.1`) and `libnurt.dylib`. The bindings for .NET and Java look for the name of the
package first and for the name of the build second, so both are found; a path given in the
options or in `NURT_LIBRARY` is used as given, whatever its name.

On musl targets (`x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`) the library of the release is static: the archive `nurt-lib-<version>-<target>`
holds `libnurt.a`, `nurt.h` and `nurt.pc`, and `pkg-config --static --libs nurt` returns every library that a static link needs. On musl the library is
static (`libnurt.a`) and must not be linked into one program with another static library written in Rust.
The program `examples/c/hello.c` calls `nurt_abi_check` and `nurt_version`; the release checks build and run it against the library archive.

## Versioning

| Item | Value |
|---|---|
| `NURT_ABI_MAJOR` | `1` |
| `NURT_ABI_MINOR` | `0` |
| `NURT_ABI_VERSION` | `(NURT_ABI_MAJOR << 16) \| NURT_ABI_MINOR` |

Within one major version the interface only grows: functions, status codes
and JSON members are added, never removed or changed. A caller passes
`NURT_ABI_VERSION` of its header to `nurt_abi_check` once after loading the
library; the call returns `NURT_OK` when the major versions are equal and the
minor version of the library is not lower, otherwise `NURT_E_ABI`.

## Conventions

- Strings passed to the library are UTF-8 and terminated by `\0`.
- Complex values are JSON documents: request envelopes `nurt-request/1`,
  responses `nurt-response/1`, syntax trees `nurt-ast/1`, configuration
  `nurt-config/1`, errors and events.
- Handles (`nurt_engine`, `nurt_exec`) are opaque.
- Every function is thread-safe unless its description says otherwise.
- After `fork()` without `exec()`, an engine opened in the parent process is
  not usable in the child; its functions return `NURT_E_INTERNAL`.

## Status codes

| Code | Value | Meaning |
|---|---|---|
| `NURT_OK` | 0 | The call succeeded. A language error, if any, is in the JSON result. |
| `NURT_E_TIMEOUT` | 1 | `nurt_exec_wait`: the execution is still running after the timeout. |
| `NURT_E_CANCELLED` | 2 | `nurt_exec_wait`: the execution ended because it was cancelled. |
| `NURT_E_ARG` | 3 | A null pointer where a value is required, text that is not UTF-8, or JSON that is not valid for the argument. |
| `NURT_E_ABI` | 4 | Incompatible interface version (`nurt_abi_check`, `nurt_ext_load`). |
| `NURT_E_IO` | 5 | A configuration file, the journal, the state directory or an extension cannot be opened or read. |
| `NURT_E_INTERNAL` | 6 | Internal failure, including a panic caught at the boundary. |

Errors of the language (syntax, binding, assertions, stale targets, journal
errors) are not call failures. The call returns `NURT_OK` and the response
carries `report.error` with a stable `E_*` code. A request envelope that is
not JSON or does not match the schema `nurt-request/1` is `NURT_E_ARG`; an
envelope that matches the schema but breaks a rule of the envelope (no input,
two inputs, `variant` without `program`, a `limit` out of range) returns
`NURT_OK` and a response with the language error `E_ARG`; a relative path
option (`base`, `save_to`) gives `E_PATH_RELATIVE`.

### Language errors in other results

| Result | Form of a language error |
|---|---|
| Response (`nurt_run_json`, `nurt_exec_wait`, `nurt_page`) | `report.error` of the response. |
| One JSON document (`nurt_correct`, `nurt_catalog`, `nurt_history`, `nurt_space`) | The document is `{"error": {...}}`. |
| Two JSON documents (`nurt_parse`, `nurt_check`) | The first document is `null`; the diagnostics are `{"diagnostics": [], "error": {...}}`. |
| Text (`nurt_print`, `nurt_describe`, `nurt_help`) and no result (`nurt_session_end`, `nurt_ext_load`) | The call fails with the status of the error code when the function may return it (`E_ARG`, `E_UNSUPPORTED`, `E_UNKNOWN_NAME`: `NURT_E_ARG`; `E_IO`: `NURT_E_IO` for `nurt_session_end` and `nurt_ext_load`), otherwise with `NURT_E_INTERNAL`. `nurt_last_error` holds the error object under `detail.error`. |

The error object has the members of the error of the effect report
(`spec/schemas/nurt-report-1.schema.json`): `code`, `message`, `at` (the position, with
`stage`, `line`, `column` and `length`), `found`, `fix`, `help`, `details`. The execution
stage is `at.stage`.

A caller treats any unknown non-zero status as a failure; a later minor
version may add codes.

### Status codes of each function

| Function | OK | TIMEOUT | CANCELLED | ARG | ABI | IO | INTERNAL |
|---|---|---|---|---|---|---|---|
| `nurt_abi_check` | x | | | | x | | |
| `nurt_info` | x | | | x | | | x |
| `nurt_engine_open` | x | | | x | | x | x |
| `nurt_engine_config_get` | x | | | x | | | x |
| `nurt_parse` | x | | | x | | | x |
| `nurt_print` | x | | | x | | | x |
| `nurt_correct` | x | | | x | | | x |
| `nurt_check` | x | | | x | | | x |
| `nurt_run_json` | x | | | x | | | x |
| `nurt_exec_start` | x | | | x | | | x |
| `nurt_exec_wait` | x | x | x | x | | | x |
| `nurt_exec_cancel` | x | | | x | | | x |
| `nurt_page` | x | | | x | | | x |
| `nurt_catalog` | x | | | x | | | x |
| `nurt_describe` | x | | | x | | | x |
| `nurt_help` | x | | | x | | | x |
| `nurt_schema` | x | | | x | | | x |
| `nurt_history` | x | | | x | | | x |
| `nurt_space` | x | | | x | | | x |
| `nurt_session_end` | x | | | x | | x | x |
| `nurt_ext_load` | x | | | x | x | x | x |
| `nurt_ext_list` | x | | | x | | | x |
| `nurt_last_error` | x | | | x | | | |

`nurt_abi_version`, `nurt_version`, `nurt_engine_close`, `nurt_exec_free`,
`nurt_buf_free` and `nurt_set_log_callback` do not return a status. The table
is verified by the interface tests in C, Python, Node and Rust, which produce
every marked status.

## Last error

`nurt_last_error(e, out_json)` returns the last failure of the calling thread:

```json
{"status": 3, "code": "NURT_E_ARG", "message": "`program` must not be null", "detail": {"function": "nurt_parse", "argument": "program"}}
```

| Member | Type | Meaning |
|---|---|---|
| `status` | integer | Value of the status code. |
| `code` | string | Name of the status code. |
| `message` | string | What failed and how to correct it. |
| `detail` | object | `function`: the exported function that failed; further members depend on the failure (`argument`, `path`, `expected`, `error`). |

The record is kept per thread, like `errno`, and is replaced only by the next
failing call of the same thread; `NURT_OK`, `NURT_E_TIMEOUT` and
`NURT_E_CANCELLED` leave it unchanged. `e` may be null, for example after a
failed `nurt_engine_open`. Without a recorded failure the buffer is empty.

## Memory

| Rule | Description |
|---|---|
| Ownership | Memory returned by the library is held in a `nurt_buf` and released with `nurt_buf_free`. The string returned by `nurt_version` is static. |
| Empty buffer | `ptr` is `NULL` exactly when `len` is 0. |
| Terminator | A `\0` byte follows the last byte and is not counted in `len`; `ptr` can be used as a C string. |
| `cap` | Belongs to the library; callers do not read it. |
| Output buffers | An output buffer is overwritten without being released: pass a buffer initialised to zero or already released. |
| Failure | When a call returns a status other than `NURT_OK`, its output buffers are empty. The response of `nurt_exec_wait` with `NURT_E_CANCELLED` is the only exception. |
| Release | `nurt_buf_free` sets the buffer to zero; it accepts `NULL` and zero buffers, so releasing twice is safe. |
| Handles | An engine is released with `nurt_engine_close`, an execution with `nurt_exec_free`; both accept `NULL`. An execution keeps the engine data it needs, so it may be released after its engine. |

## Engine

`nurt_engine_open` opens the engine with the clock and the host of the
process. The configuration is assembled from the built-in defaults, the
system and user files, the project file, the file named by `NURT_CONFIG`,
the `NURT_*` variables and run-time settings; `config_json` is applied at the
level of the `NURT_CONFIG` file, after that file. The state directory is
`state.dir` or the directory of the platform.

| Failure | Status |
|---|---|
| `config_json` is not JSON or not an object `nurt-config/1` | `NURT_E_ARG` |
| A value does not match its key, for example a relative `state.dir` | `NURT_E_ARG`; `detail.error.code` is `E_CONFIG` or `E_PATH_RELATIVE` |
| A configuration file or the state directory cannot be read or created | `NURT_E_IO` |

| Function | Result |
|---|---|
| `nurt_engine_config_get` | The effective configuration `nurt-config/1`. |
| `nurt_parse` | The syntax tree and `{"diagnostics": []}`. |
| `nurt_correct` | `{"program": text, "ast": tree, "corrections": [...]}`; `program` is the corrected text, or the given text when nothing was corrected. |
| `nurt_check` | The check document `nurt-check/1` (`format`, `effect`, `reversibility`, `operations`, `waits`, `paths`, `network`, `programs`, `groups`, `corrections`, `details`) with the member `bound` holding the bound tree, and `{"diagnostics": []}`. |
| `nurt_page` | The response of the page; `limit` 0 keeps the limit of the original request. |
| `nurt_describe` | Text of the subject `name`, else `layer`, else `core`, in `language` or the configured language. Members other than `layer`, `name` and `language` are `NURT_E_ARG`. |
| `nurt_help` | Text of the topic; `NULL` asks for the index of topics (the empty topic). |
| `nurt_history` | `{"operations": [...]}`; `query_json` holds the parameters of the command `history` (`id`, `limit`, `state`, `before`, `since`, `path`, `deleted_only`); an unknown member is `NURT_E_ARG`. |
| `nurt_space` | `{"volumes": [...]}`; a relative `volume` is the language error `E_PATH_RELATIVE`. |
| `nurt_session_end` | Removes the run-time settings of the session. An identifier that is empty, longer than the limit or with control characters is `NURT_E_ARG`. |
| `nurt_ext_list` | `{"extensions": [...]}`. |
| `nurt_ext_load` | `E_UNSUPPORTED` (`NURT_E_ARG`) when the library has no extension host. |
| `nurt_catalog` | The command catalog, or `{"error": ...}` with `E_UNSUPPORTED` while the engine has no catalog. `what_json` is `null`, `{"group": name}` for the commands of a group or `{"command": name}` for the specification of a command; another member is `NURT_E_ARG`, an unknown name is `{"error": ...}`. |

## Information

`nurt_info` needs no engine and returns:

| Member | Meaning |
|---|---|
| `version` | Version of the library. |
| `abi` | `major`, `minor` and `version` of the interface. |
| `formats` | Versions of the formats `ast`, `bound`, `report`, `request`, `response`. |
| `schemas` | Names accepted by `nurt_schema`. |
| `catalog_rev` | Revision of the command catalog built into the library: `sha256:` and the digest of its canonical JSON; `null` when the catalog cannot be loaded. |
| `features` | Cargo features compiled into the library, for example `stub`. |
| `target` | `arch`, `os`, `family`, `pointer_width`, `endian`. |
| `build` | `profile`: `debug` or `release`. |

## Panics

Every exported function runs inside a panic boundary. A panic is caught,
reported as `NURT_E_INTERNAL` with the panic message in the last error, and
sent to the log receiver at level `NURT_LOG_ERROR`. The engine stays usable.
The boundary relies on unwinding: the library is built with
`panic = "unwind"`.

## Threads

| Item | Behaviour |
|---|---|
| Engine | One engine may be used from several threads at once. Several engines may exist in one process. |
| Synchronous calls | `nurt_run_json` and the other functions with a result run on the calling thread. |
| Execution | `nurt_exec_start` runs the request on a thread of the engine and returns at once. |
| Events | Each execution has one delivery thread. The event callback is called from that thread only, one event at a time. Events are buffered, so a slow callback does not slow the execution. |
| Log receiver | `nurt_set_log_callback` sets one receiver for the whole process; it may be called from any thread. Passing `NULL` stops logging. |
| Last error | Kept per thread. |

### Events of an execution

Each event is one JSON object passed to the callback as `event_json`, valid
only during the call:

```json
{"seq": 1, "time": "2026-11-21T09:12:00Z", "request": "r1", "t": "exec.started"}
{"seq": 2, "time": "2026-11-21T09:12:00Z", "request": "r1", "t": "progress", "command": "find", "phase": "scan", "done": 64, "total": 148, "bytes": null, "message": null}
{"seq": 3, "time": "2026-11-21T09:12:00Z", "request": "r1", "t": "exec.finished", "status": "ok", "error": null}
```

The members are those of `spec/schemas/nurt-event-1.schema.json`, the same document that
`nurt serve` sends in the notifications `nurt/event`.

| Member | Meaning |
|---|---|
| `seq` | Sequence number, from 1, increasing by 1 within the execution. |
| `time` | Time of the event as text, RFC 3339 in UTC. |
| `request` | Identifier of the request as text, the same for every event of the execution; `null` when the request has none. |
| `t` | Event type: `exec.started`, `progress`, `job.started`, `job.output`, `job.progress`, `job.finished`, `log`, `exec.finished`. |
| `id` | `job.*` events only: the identifier of the background job. |
| `status` | `exec.finished` only: the status of the report (`ok`, `failed` and the other values of `report.status`). An execution that ends without a response has `failed`. |
| `error` | `exec.finished` only: the code of the error that ended the execution, or `null`. |

The first event is `exec.started` and the last is `exec.finished`. No callback
follows `exec.finished`, and none follows `nurt_exec_free`. The callback may
call other functions of the library; it does not call `nurt_exec_free` for its
own execution, `nurt_engine_close` for its engine, or `nurt_exec_wait` for its
own execution with a timeout other than 0.

## Waiting and cancellation

| Call | Behaviour |
|---|---|
| `nurt_exec_wait(x, 0, out)` | Checks without waiting: `NURT_E_TIMEOUT` and an empty buffer while the execution runs. |
| `nurt_exec_wait(x, ms, out)` | Waits for no longer than `ms` milliseconds. |
| `nurt_exec_wait(x, NURT_WAIT_FOREVER, out)` | Waits without limit. |
| After completion | Every call returns the same status and the same response. |
| `nurt_exec_cancel(x)` | Requests cancellation and returns `NURT_OK`. Repeated calls have no further effect. |
| Cancellation of a running execution | The execution stops at the next cancellation point. `nurt_exec_wait` returns `NURT_E_CANCELLED` and a response with status `failed` and the error `E_CANCELLED`. Changes already committed stay in the journal and can be undone. |
| Cancellation after completion | Has no effect: the result stays `NURT_OK` or the failure status the execution ended with. |
| `nurt_exec_free(x)` | Cancels a running execution, waits for its threads and releases the handle. |
| `nurt_engine_close(e)` | Cancels the running executions of the engine and waits until they complete. |

An execution is complete once `exec.finished` has been delivered, so after
`nurt_exec_wait` returns a result no event callback runs for the execution.
`nurt_exec_wait` returns `NURT_E_CANCELLED` exactly when the response reports
`E_CANCELLED`; an execution that ended before the cancellation point keeps its
response and `NURT_OK`.

## Test engine

With the cargo feature `stub` the engine is replaced by a test double that
answers from recorded cases; its events have the same members as the events
of the engine. The feature is never enabled in a release
build. Each case is a JSON file in the directory named by `NURT_STUB_DIR`
(by default `crates/nurt-ffi/tests/fixtures/stub`); a call is answered by
the case whose key equals the SHA-256 digest of the canonical JSON form
(RFC 8785) of `{"function": ..., "input": ...}`. `NURT_STUB_PANIC` holds a
comma-separated list of exported functions that panic inside their boundary,
for tests of `NURT_E_INTERNAL`.

## Examples

Running a request:

```c
#include <stdio.h>
#include "nurt.h"

int main(void) {
    nurt_engine *engine = NULL;
    nurt_buf response = {0};
    if (nurt_abi_check(NURT_ABI_VERSION) != NURT_OK ||
        nurt_engine_open(NULL, &engine) != NURT_OK) {
        nurt_buf error = {0};
        nurt_last_error(NULL, &error);
        fprintf(stderr, "%s\n", error.ptr ? (const char *)error.ptr : "unknown failure");
        nurt_buf_free(&error);
        return 1;
    }
    if (nurt_run_json(engine, "{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\"\"}",
                      &response) == NURT_OK) {
        printf("%s\n", (const char *)response.ptr);
    }
    nurt_buf_free(&response);
    nurt_engine_close(engine);
    return 0;
}
```

Waiting for an execution with a deadline:

```c
nurt_exec *exec = NULL;
nurt_buf response = {0};
nurt_exec_start(engine, request_json, on_event, &state, &exec);
if (nurt_exec_wait(exec, 5000, &response) == NURT_E_TIMEOUT) {
    nurt_exec_cancel(exec);
    nurt_exec_wait(exec, NURT_WAIT_FOREVER, &response);
}
nurt_buf_free(&response);
nurt_exec_free(exec);
```
