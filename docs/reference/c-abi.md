<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
# C interface (libnurt)

The library `libnurt` exposes the engine to C and to every language with a C
foreign function interface. The interface is declared in
`crates/nurt-ffi/include/nurt.h`; the exported symbols are listed in
`crates/nurt-ffi/nurt.symbols`. The crate `nurt-ffi` builds a shared library
(`libnurt_ffi.so`, `libnurt_ffi.dylib`, `nurt_ffi.dll`) and a static library.

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
two inputs, `variant` without `program`, a `limit` out of range, a relative
`base`) returns `NURT_OK` and a response with the language error `E_ARG`.

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
| `detail` | object | `function`: the exported function that failed; further members depend on the failure (`argument`, `path`, `expected`). |

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
| Execution | `nurt_exec_start` runs the request on a thread of the library and returns at once. |
| Events | Each execution has one delivery thread. The event callback is called from that thread only, one event at a time. Events are buffered, so a slow callback does not slow the execution. |
| Log receiver | `nurt_set_log_callback` sets one receiver for the whole process; it may be called from any thread. Passing `NULL` stops logging. |
| Last error | Kept per thread. |

### Events of an execution

Each event is one JSON object passed to the callback as `event_json`, valid
only during the call:

```json
{"seq": 1, "t": "exec.started"}
{"seq": 2, "t": "progress", "command": "find", "phase": "scan", "done": 64, "total": 148, "bytes": null, "message": null}
{"seq": 3, "t": "exec.finished", "status": 0}
```

| Member | Meaning |
|---|---|
| `seq` | Sequence number, from 1, increasing by 1 within the execution. |
| `t` | Event type: `exec.started`, `progress`, `job.started`, `job.output`, `job.progress`, `job.finished`, `log`, `exec.finished`. |
| `status` | `exec.finished` only: the status that `nurt_exec_wait` returns for the execution. |

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

## Test engine

With the cargo feature `stub` the engine is replaced by a test double that
answers from recorded cases. The feature is never enabled in a release
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
