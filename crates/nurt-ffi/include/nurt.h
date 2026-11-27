// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-ffi/include/nurt.h
// Responsibility:   Source file crates/nurt-ffi/include/nurt.h.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

/*
 * nurt.h - C interface of the Danaco Nurt library (libnurt).
 *
 * This header is maintained by hand; the list of exported symbols is part of
 * the stable interface.
 *
 * Conventions
 * -----------
 * - Every string is UTF-8 and terminated by '\0'.
 * - Complex values cross the boundary as JSON: syntax trees nurt-ast/1, bound
 *   trees nurt-bound/1, request envelopes nurt-request/1, responses,
 *   configuration nurt-config/1, errors and events.
 * - Memory returned by the library is held in a nurt_buf and released with
 *   nurt_buf_free. Strings returned as `const char *` (nurt_version) are
 *   static and are never released.
 * - Handles (nurt_engine, nurt_exec) are opaque.
 * - Every function is thread-safe unless its description says otherwise.
 * - No panic crosses the boundary: an internal failure returns
 *   NURT_E_INTERNAL and the engine stays usable when possible.
 * - After fork() without exec() an engine created in the parent process is
 *   not usable in the child; its functions return NURT_E_INTERNAL.
 *
 * Status codes and language errors
 * --------------------------------
 * nurt_status reports only failures of the call itself. Errors of the
 * language (syntax, binding, assertions, stale targets, ambiguity, journal
 * errors and so on) are not call failures: the call returns NURT_OK and the
 * JSON result carries an `error` object with a stable `E_*` code, the stage,
 * the position and a fix. Codes 0 to 6 are stable within an ABI major
 * version; a later minor version may add codes, so a caller treats any
 * unknown non-zero value as a failure.
 *
 * Possible status codes of each function
 * --------------------------------------
 *   function                  OK  TIMEOUT  CANCELLED  ARG  ABI  IO  INTERNAL
 *   nurt_abi_check            x                            x
 *   nurt_info                 x                       x             x
 *   nurt_engine_open          x                       x         x   x
 *   nurt_engine_config_get    x                       x             x
 *   nurt_parse                x                       x             x
 *   nurt_print                x                       x             x
 *   nurt_correct              x                       x             x
 *   nurt_check                x                       x             x
 *   nurt_run_json             x                       x             x
 *   nurt_exec_start           x                       x             x
 *   nurt_exec_wait            x   x        x          x             x
 *   nurt_exec_cancel          x                       x             x
 *   nurt_page                 x                       x             x
 *   nurt_catalog              x                       x             x
 *   nurt_describe             x                       x             x
 *   nurt_help                 x                       x             x
 *   nurt_schema               x                       x             x
 *   nurt_history              x                       x             x
 *   nurt_space                x                       x             x
 *   nurt_session_end          x                       x         x   x
 *   nurt_ext_load             x                       x    x    x   x
 *   nurt_ext_list             x                       x             x
 *   nurt_last_error           x                       x
 * nurt_abi_version, nurt_version, nurt_engine_close, nurt_exec_free,
 * nurt_buf_free and nurt_set_log_callback do not return a status.
 *
 * NURT_E_ARG means a null pointer where a value is required, text that is
 * not valid UTF-8, or JSON that is not valid for the argument (for example a
 * request envelope that is not an object). Details of the last failure of
 * the calling thread are returned by nurt_last_error.
 */

#ifndef NURT_H
#define NURT_H

#include <stddef.h>
#include <stdint.h>

/* Version of the interface described by this header. Within one major
 * version the interface only grows: functions, status codes and JSON fields
 * are added, never removed or changed. */
#define NURT_ABI_MAJOR 1
#define NURT_ABI_MINOR 0

/* Version of this header in the form returned by nurt_abi_version. */
#define NURT_ABI_VERSION (((uint32_t)NURT_ABI_MAJOR << 16) | (uint32_t)NURT_ABI_MINOR)

/* Timeout of nurt_exec_wait that waits without limit. */
#define NURT_WAIT_FOREVER UINT32_MAX

/* Visibility and calling convention of exported functions. Define
 * NURT_STATIC when linking the static library on Windows. */
#if defined(_WIN32) || defined(__CYGWIN__)
#  if defined(NURT_STATIC)
#    define NURT_API
#  else
#    define NURT_API __declspec(dllimport)
#  endif
#  define NURT_CALL __cdecl
#elif defined(__GNUC__) || defined(__clang__)
#  define NURT_API __attribute__((visibility("default")))
#  define NURT_CALL
#else
#  define NURT_API
#  define NURT_CALL
#endif

#ifdef __cplusplus
extern "C" {
#endif

/* Engine instance: configuration, command catalog, journal and sessions.
 * Created by nurt_engine_open and released by nurt_engine_close. Several
 * engines may exist in one process, and several processes may use the same
 * state directory; the journal serialises their changes with file locks. */
typedef struct nurt_engine nurt_engine;

/* Asynchronous execution started by nurt_exec_start and released by
 * nurt_exec_free. */
typedef struct nurt_exec nurt_exec;

/* Buffer returned by the library: UTF-8 text or JSON.
 *
 * - `ptr` is NULL exactly when `len` is 0.
 * - A '\0' byte follows the last byte; it is not counted in `len`, so `ptr`
 *   can be used as a C string.
 * - `cap` belongs to the library and is not read by callers.
 * - An output buffer is overwritten without being released: pass a buffer
 *   initialised to zero or already released with nurt_buf_free.
 * - When a call returns a status other than NURT_OK, its output buffers are
 *   left empty (except the response of nurt_exec_wait with
 *   NURT_E_CANCELLED). */
typedef struct nurt_buf {
    uint8_t *ptr;
    size_t len;
    size_t cap;
} nurt_buf;

/* Result of a call of the library. */
typedef enum nurt_status {
    /* The call succeeded; a language error, if any, is in the JSON result. */
    NURT_OK = 0,
    /* nurt_exec_wait: the execution is still running after the timeout. */
    NURT_E_TIMEOUT = 1,
    /* nurt_exec_wait: the execution ended because nurt_exec_cancel was
     * called; the response buffer holds the report of the cancelled
     * execution. */
    NURT_E_CANCELLED = 2,
    /* Invalid argument: null pointer, invalid UTF-8 or invalid JSON. */
    NURT_E_ARG = 3,
    /* Incompatible interface version (nurt_abi_check, extension interface
     * of nurt_ext_load). */
    NURT_E_ABI = 4,
    /* A configuration file, the journal, the state directory or an
     * extension cannot be opened or read. */
    NURT_E_IO = 5,
    /* Internal failure, including a panic caught at the boundary. */
    NURT_E_INTERNAL = 6
} nurt_status;

/* Notation variant of the text form of programs. */
typedef enum nurt_variant {
    /* Word notation. */
    NURT_VARIANT_A = 1,
    /* Call notation. */
    NURT_VARIANT_B = 2,
    /* Named notation. */
    NURT_VARIANT_C = 3
} nurt_variant;

/* Level of a diagnostic log message. */
typedef enum nurt_log_level {
    NURT_LOG_ERROR = 1,
    NURT_LOG_WARN = 2,
    NURT_LOG_INFO = 3,
    NURT_LOG_DEBUG = 4,
    NURT_LOG_TRACE = 5
} nurt_log_level;

/* Receives the events of one execution (progress, output of programs,
 * journal changes, completion) as a JSON object.
 *
 * - The callback is called from one thread of the library per execution,
 *   one event at a time, in increasing `seq`; the last event is
 *   `exec.finished`, and no call follows it or nurt_exec_free.
 * - `event_json` is valid only during the call.
 * - The callback may call other functions of the library, but not
 *   nurt_exec_free for the same execution. A slow callback does not block the
 *   execution: events are buffered and program output is merged, without loss
 *   (complete output remains available through a cursor). */
typedef void (*nurt_event_cb)(void *user, const char *event_json);

/* Receives diagnostic log messages; `level` is a nurt_log_level value and
 * `msg` is valid only during the call. Called from any thread. */
typedef void (*nurt_log_cb)(void *user, int level, const char *msg);

/* Version of the interface implemented by the library:
 * (NURT_ABI_MAJOR << 16) | NURT_ABI_MINOR. */
NURT_API uint32_t NURT_CALL nurt_abi_version(void);

/* Checks that the library implements the interface of the caller's header.
 * Pass NURT_ABI_VERSION. Returns NURT_OK when the major versions are equal
 * and the minor version of the library is not lower, otherwise NURT_E_ABI. */
NURT_API nurt_status NURT_CALL nurt_abi_check(uint32_t header_version);

/* Version of the library, for example "0.1.0". The string is static. */
NURT_API const char *NURT_CALL nurt_version(void);

/* Information about the library without an engine, as JSON: version,
 * interface version, language and format versions, catalog revision,
 * features, target and build. */
NURT_API nurt_status NURT_CALL nurt_info(nurt_buf *out_json);

/* Opens an engine.
 *
 * `config_json` is NULL or a JSON object in the format nurt-config/1. The
 * configuration is assembled from the built-in defaults, the system and user
 * files, the project file, the file named by NURT_CONFIG, NURT_* variables
 * and run-time settings; `config_json` is applied at the level of the
 * NURT_CONFIG file, after that file when both are present. It may set
 * protection floors of the journal and trusted projects.
 *
 * NURT_E_ARG: `out` is NULL or `config_json` is not a valid configuration.
 * NURT_E_IO: a configuration file or the state directory cannot be read. */
NURT_API nurt_status NURT_CALL nurt_engine_open(const char *config_json, nurt_engine **out);

/* Closes an engine. Running executions are cancelled and waited for. NULL is
 * accepted. The engine must not be used after the call. */
NURT_API void NURT_CALL nurt_engine_close(nurt_engine *e);

/* Effective configuration of the engine as JSON (nurt-config/1). */
NURT_API nurt_status NURT_CALL nurt_engine_config_get(nurt_engine *e, nurt_buf *out_json);

/* Parses a program in the given notation into a syntax tree nurt-ast/1.
 * Diagnostics (positions, messages) are written to `out_diag_json`. No
 * effect on the world. */
NURT_API nurt_status NURT_CALL nurt_parse(nurt_engine *e, const char *program, nurt_variant v,
                                          nurt_buf *out_ast_json, nurt_buf *out_diag_json);

/* Prints a syntax tree nurt-ast/1 in the canonical text of the given
 * notation. No effect on the world. */
NURT_API nurt_status NURT_CALL nurt_print(nurt_engine *e, const char *ast_json, nurt_variant v,
                                          nurt_buf *out_text);

/* Corrects the written form of a program without running it; the result
 * holds the corrected program and the list of corrections. */
NURT_API nurt_status NURT_CALL nurt_correct(nurt_engine *e, const char *program, nurt_variant v,
                                            nurt_buf *out_result_json);

/* Binds a request envelope nurt-request/1 and classifies its effect without
 * running it: bound tree, targets, paths, external programs, network
 * addresses, corrections and notable effects. Embedding applications use the
 * result to decide whether to run the request. */
NURT_API nurt_status NURT_CALL nurt_check(nurt_engine *e, const char *request_json,
                                          nurt_buf *out_bound_json, nurt_buf *out_diag_json);

/* Runs a request envelope nurt-request/1 and waits for the response. */
NURT_API nurt_status NURT_CALL nurt_run_json(nurt_engine *e, const char *request_json,
                                             nurt_buf *out_response_json);

/* Starts a request envelope nurt-request/1 asynchronously. `cb` may be NULL
 * when no events are needed. The handle is released with nurt_exec_free. */
NURT_API nurt_status NURT_CALL nurt_exec_start(nurt_engine *e, const char *request_json,
                                               nurt_event_cb cb, void *user, nurt_exec **out);

/* Waits for an execution.
 *
 * `timeout_ms` 0 checks without waiting; NURT_WAIT_FOREVER waits without
 * limit. Returns NURT_OK with the response when the execution has finished,
 * NURT_E_TIMEOUT when it is still running (the buffer stays empty; wait
 * again), and NURT_E_CANCELLED with the response of the cancelled execution
 * after nurt_exec_cancel. May be called again after completion and returns
 * the same result. */
NURT_API nurt_status NURT_CALL nurt_exec_wait(nurt_exec *x, uint32_t timeout_ms,
                                              nurt_buf *out_response_json);

/* Requests cancellation of an execution. Idempotent; returns NURT_OK also
 * when the execution has already finished. Changes already committed stay
 * in the journal and can be undone. */
NURT_API nurt_status NURT_CALL nurt_exec_cancel(nurt_exec *x);

/* Releases an execution handle. A running execution is cancelled first.
 * NULL is accepted. Must not be called from the event callback of the same
 * execution. */
NURT_API void NURT_CALL nurt_exec_free(nurt_exec *x);

/* Returns the next page of a paged result; `limit` 0 uses the limit of the
 * original request. An expired cursor is a language error in the response. */
NURT_API nurt_status NURT_CALL nurt_page(nurt_engine *e, const char *cursor, uint32_t limit,
                                         nurt_buf *out_response_json);

/* Command catalog as JSON. `what_json` is NULL for the whole catalog or an
 * object selecting a group or a command. */
NURT_API nurt_status NURT_CALL nurt_catalog(nurt_engine *e, const char *what_json,
                                            nurt_buf *out_json);

/* Description of the language for tool integrations, as text. `what_json`
 * selects the layer, the group and the language, for example
 * {"layer":"group","name":"documents","language":"en"}; NULL returns the core
 * description in the configured language. */
NURT_API nurt_status NURT_CALL nurt_describe(nurt_engine *e, const char *what_json,
                                             nurt_buf *out_text);

/* Built-in help on a topic, as text; NULL returns the index of topics. */
NURT_API nurt_status NURT_CALL nurt_help(nurt_engine *e, const char *topic, nurt_buf *out_text);

/* Published JSON schema by name: "ast", "bound", "report", "request",
 * "response", "config", "catalog", "catalog-result", "value", "tool",
 * "fs-snapshot", "check", "rpc" or "event". An unknown name returns
 * NURT_E_ARG. Needs no engine. */
NURT_API nurt_status NURT_CALL nurt_schema(const char *which, nurt_buf *out_json);

/* Journal history as JSON; `query_json` is NULL or an object with the
 * parameters of the command `history`. */
NURT_API nurt_status NURT_CALL nurt_history(nurt_engine *e, const char *query_json,
                                            nurt_buf *out_json);

/* Free space and journal usage of a volume as JSON; `volume` is a path on
 * the volume, or NULL for all volumes. */
NURT_API nurt_status NURT_CALL nurt_space(nurt_engine *e, const char *volume, nurt_buf *out_json);

/* Ends a session: removes the run-time settings recorded for the session
 * identifier (for example limits set by `journal.limits` with session
 * scope). Ending an unknown session succeeds. */
NURT_API nurt_status NURT_CALL nurt_session_end(nurt_engine *e, const char *session);

/* Loads an extension from its manifest nurt-ext.toml. NURT_E_ABI: the
 * extension needs another major version of the extension interface.
 * NURT_E_IO: the manifest or the module cannot be read. An invalid manifest
 * is NURT_E_ARG. */
NURT_API nurt_status NURT_CALL nurt_ext_load(nurt_engine *e, const char *manifest_path);

/* Loaded extensions as JSON. */
NURT_API nurt_status NURT_CALL nurt_ext_list(nurt_engine *e, nurt_buf *out_json);

/* Releases a buffer returned by the library and sets it to zero. Idempotent;
 * NULL and zero buffers are accepted. */
NURT_API void NURT_CALL nurt_buf_free(nurt_buf *b);

/* Last failure of the calling thread as JSON:
 * {"status": 3, "code": "NURT_E_ARG", "message": "...", "detail": {...}}.
 * The record is kept per thread, like errno, and is replaced by the next
 * failing call of the thread. `e` may be NULL, for example after a failed
 * nurt_engine_open. Without a recorded failure the buffer is empty. */
NURT_API nurt_status NURT_CALL nurt_last_error(nurt_engine *e, nurt_buf *out_json);

/* Sets the receiver of diagnostic log messages for the whole process; NULL
 * stops logging. Messages are filtered by the configured log level. */
NURT_API void NURT_CALL nurt_set_log_callback(nurt_log_cb cb, void *user);

#ifdef __cplusplus
}
#endif

#endif /* NURT_H */

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
