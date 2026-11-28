<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/architecture/c-interface.md -->
<!-- Responsibility:   Documents C interface. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# C interface

The library `libnurt` (crate `nurt-ffi`) exposes the engine to any language
through the functions declared in `crates/nurt-ffi/include/nurt.h`. The
header is maintained by hand and is the reference of the interface.

## Artifacts

| System | Shared library | Static library |
|---|---|---|
| Linux | `libnurt.so.1` (SONAME, major version of the interface) | `libnurt.a` |
| macOS | `libnurt.1.dylib` (install name `@rpath/libnurt.1.dylib`) | `libnurt.a` |
| Windows | `nurt.dll` with the import library `nurt.dll.lib` | `nurt.lib` (define `NURT_STATIC`) |

On musl targets the release archive of the library holds only the static library `libnurt.a`. On musl the library is static
(`libnurt.a`) and must not be linked into one program with another static library written in Rust.

## Versioning

- `NURT_ABI_MAJOR` and `NURT_ABI_MINOR` give the version of the header;
  `nurt_abi_version()` returns the version of the library as
  `(major << 16) | minor`.
- A binding calls `nurt_abi_check(NURT_ABI_VERSION)` once; it fails with
  `NURT_E_ABI` when the major versions differ or the library is older than
  the header.
- Within one major version the interface only grows: functions, status codes
  and JSON fields are added; nothing is removed or changed.

## Data and memory

- Strings are UTF-8 and terminated by `'\0'`; complex values are JSON in the
  published formats (`nurt-ast/1`, `nurt-bound/1`, `nurt-request/1`,
  `nurt-config/1`, responses, events, errors).
- Results are returned in `nurt_buf { ptr, len, cap }`: `ptr` is NULL exactly
  when `len` is 0, a `'\0'` byte follows the data, `cap` is private, and the
  buffer is released with `nurt_buf_free`, which is idempotent and accepts
  NULL.
- When a call fails, its output buffers stay empty. The only exception is
  `nurt_exec_wait` with `NURT_E_CANCELLED`, which returns the report of the
  cancelled execution.

## Status codes

`nurt_status` reports only failures of the call. Errors of the language are
returned with `NURT_OK` inside the JSON result (`error` with an `E_*` code).

| Code | Value | Meaning |
|---|---|---|
| `NURT_OK` | 0 | the call succeeded |
| `NURT_E_TIMEOUT` | 1 | `nurt_exec_wait`: still running |
| `NURT_E_CANCELLED` | 2 | `nurt_exec_wait`: ended by `nurt_exec_cancel` |
| `NURT_E_ARG` | 3 | null pointer, invalid UTF-8 or invalid JSON argument |
| `NURT_E_ABI` | 4 | incompatible interface version |
| `NURT_E_IO` | 5 | configuration, journal, state directory or extension cannot be read |
| `NURT_E_INTERNAL` | 6 | internal failure, including a panic caught at the boundary |

The possible codes of every function are listed at the top of `nurt.h`.
Details of the last failure of the calling thread are returned by
`nurt_last_error`, which works also without an engine.

## Threads and callbacks

- Every function is thread-safe; several engines may exist in one process.
- Events of an execution are delivered to `nurt_event_cb` from one library
  thread per execution, one at a time, in increasing `seq`, ending with
  `exec.finished`. The callback must not free its own execution.
- Diagnostic messages go to the process-wide `nurt_set_log_callback`
  receiver with a `nurt_log_level`.
- No panic crosses the boundary. After `fork()` without `exec()`, an engine
  of the parent process is not usable in the child.

## Configuration

`nurt_engine_open(config_json, &engine)` accepts `NULL` or a JSON object in
the format `nurt-config/1` (see `spec/configuration.md`). It is applied at the
level of the `NURT_CONFIG` file, so an embedding application can set trusted
projects and protection floors of the journal; run-time settings cannot
override trust.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
