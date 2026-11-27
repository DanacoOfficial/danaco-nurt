// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/python/tests/mock/mock_libnurt.c
// Responsibility:   Source file bindings/python/tests/mock/mock_libnurt.c.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

/*
 * Mock of libnurt for the tests of the Python binding.
 *
 * Implements every function of nurt.h with fixed behaviour selected by the
 * first word of the program:
 *
 *   fail          language error E_SYNTAX in the response
 *   sleep <ms>    waits <ms> milliseconds (cancellable when started)
 *   events <n>    emits <n> job.progress events when started
 *   page          response with next_cursor "page-2"
 *   options       rendered text holds the options object of the request
 *   status <n>    the call fails with nurt_status <n>
 *   panic         the call fails with NURT_E_INTERNAL
 *   anything else response "ok" with the program as rendered text
 *
 * Functions prefixed nurt_mock_ expose counters and failure injection to the
 * tests. NURT_MOCK_ABI ("major.minor") overrides the reported interface
 * version.
 */

#define _POSIX_C_SOURCE 200809L

#include <errno.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#include "nurt.h"

#define MOCK_EXPORT __attribute__((visibility("default")))
#define SLICE_MS 5
#define ERROR_CAPACITY 512

struct nurt_engine {
    pid_t pid;
};

struct nurt_exec {
    pthread_t thread;
    pthread_mutex_t lock;
    pthread_cond_t done;
    nurt_event_cb cb;
    void *user;
    char *program;
    char *response;
    int finished;
    int cancelled;
    nurt_status status;
};

struct text {
    char *data;
    size_t len;
    size_t cap;
};

static atomic_long live_buffers;
static atomic_long live_executions;
static atomic_long live_engines;

static pthread_mutex_t forced_lock = PTHREAD_MUTEX_INITIALIZER;
static char forced_function[64];
static int forced_status;

static pthread_mutex_t log_lock = PTHREAD_MUTEX_INITIALIZER;
static nurt_log_cb log_receiver;
static void *log_user;

static _Thread_local char last_error[ERROR_CAPACITY];
static _Thread_local int last_error_set;

static const char *const status_names[] = {
    "NURT_OK", "NURT_E_TIMEOUT", "NURT_E_CANCELLED", "NURT_E_ARG",
    "NURT_E_ABI", "NURT_E_IO", "NURT_E_INTERNAL",
};

MOCK_EXPORT long nurt_mock_live_buffers(void) { return atomic_load(&live_buffers); }
MOCK_EXPORT long nurt_mock_live_executions(void) { return atomic_load(&live_executions); }
MOCK_EXPORT long nurt_mock_live_engines(void) { return atomic_load(&live_engines); }

MOCK_EXPORT void nurt_mock_fail_next(const char *function, int status) {
    pthread_mutex_lock(&forced_lock);
    snprintf(forced_function, sizeof forced_function, "%s", function ? function : "");
    forced_status = status;
    pthread_mutex_unlock(&forced_lock);
}

static nurt_status fail(nurt_status status, const char *message) {
    const char *name = (unsigned)status < 7 ? status_names[status] : "NURT_E_UNKNOWN";
    snprintf(last_error, sizeof last_error,
             "{\"status\":%d,\"code\":\"%s\",\"message\":\"%s\",\"detail\":{}}", (int)status, name,
             message);
    last_error_set = 1;
    return status;
}

static int take_forced(const char *function) {
    int status = 0;
    pthread_mutex_lock(&forced_lock);
    if (forced_status != 0 && strcmp(forced_function, function) == 0) {
        status = forced_status;
        forced_status = 0;
        forced_function[0] = '\0';
    }
    pthread_mutex_unlock(&forced_lock);
    return status;
}

#define FORCED(function)                                                   \
    do {                                                                   \
        int forced_ = take_forced(function);                               \
        if (forced_ != 0) return fail((nurt_status)forced_, "forced failure"); \
    } while (0)

static void emit_log(int level, const char *message) {
    pthread_mutex_lock(&log_lock);
    nurt_log_cb receiver = log_receiver;
    void *user = log_user;
    pthread_mutex_unlock(&log_lock);
    if (receiver) receiver(user, level, message);
}

static void text_reserve(struct text *t, size_t extra) {
    if (t->len + extra + 1 <= t->cap) return;
    size_t cap = t->cap ? t->cap : 64;
    while (cap < t->len + extra + 1) cap *= 2;
    char *data = realloc(t->data, cap);
    if (!data) abort();
    t->data = data;
    t->cap = cap;
}

static void text_add(struct text *t, const char *s, size_t n) {
    text_reserve(t, n);
    memcpy(t->data + t->len, s, n);
    t->len += n;
    t->data[t->len] = '\0';
}

static void text_put(struct text *t, const char *s) { text_add(t, s, strlen(s)); }

static void text_json_string(struct text *t, const char *s) {
    text_put(t, "\"");
    for (; *s; ++s) {
        char escaped[8];
        unsigned char c = (unsigned char)*s;
        if (c == '"' || c == '\\') {
            escaped[0] = '\\';
            escaped[1] = (char)c;
            text_add(t, escaped, 2);
        } else if (c < 0x20) {
            snprintf(escaped, sizeof escaped, "\\u%04x", c);
            text_put(t, escaped);
        } else {
            text_add(t, (const char *)&c, 1);
        }
    }
    text_put(t, "\"");
}

static void put(nurt_buf *out, const char *data, size_t len) {
    if (!out) return;
    if (len == 0) {
        out->ptr = NULL;
        out->len = 0;
        out->cap = 0;
        return;
    }
    uint8_t *copy = malloc(len + 1);
    if (!copy) abort();
    memcpy(copy, data, len);
    copy[len] = '\0';
    out->ptr = copy;
    out->len = len;
    out->cap = len + 1;
    atomic_fetch_add(&live_buffers, 1);
}

static void put_text(nurt_buf *out, struct text *t) {
    put(out, t->data ? t->data : "", t->len);
    free(t->data);
    t->data = NULL;
    t->len = t->cap = 0;
}

static void put_string(nurt_buf *out, const char *s) { put(out, s, strlen(s)); }

/* Returns a copy of the string value of "key" in a compact JSON object, or NULL. */
static char *json_string_field(const char *json, const char *key) {
    char pattern[64];
    snprintf(pattern, sizeof pattern, "\"%s\":\"", key);
    const char *start = json ? strstr(json, pattern) : NULL;
    if (!start) return NULL;
    start += strlen(pattern);
    struct text value = {0};
    for (const char *p = start; *p && *p != '"'; ++p) {
        char c = *p;
        if (c == '\\' && p[1]) {
            ++p;
            switch (*p) {
            case 'n': c = '\n'; break;
            case 't': c = '\t'; break;
            case 'r': c = '\r'; break;
            case 'u': c = '?'; p += 4; break;
            default: c = *p; break;
            }
        }
        text_add(&value, &c, 1);
    }
    if (!value.data) text_put(&value, "");
    return value.data;
}

/* Returns a copy of the object value of "key", assuming no braces inside strings. */
static char *json_object_field(const char *json, const char *key) {
    char pattern[64];
    snprintf(pattern, sizeof pattern, "\"%s\":{", key);
    const char *start = json ? strstr(json, pattern) : NULL;
    if (!start) return NULL;
    start += strlen(pattern) - 1;
    int depth = 0;
    const char *p = start;
    for (; *p; ++p) {
        if (*p == '{') ++depth;
        if (*p == '}' && --depth == 0) break;
    }
    size_t len = (size_t)(p - start) + (*p ? 1 : 0);
    char *copy = malloc(len + 1);
    if (!copy) abort();
    memcpy(copy, start, len);
    copy[len] = '\0';
    return copy;
}

static int starts_with(const char *s, const char *prefix) {
    return strncmp(s, prefix, strlen(prefix)) == 0;
}

static void sleep_ms(long ms) {
    struct timespec ts = {ms / 1000, (ms % 1000) * 1000000L};
    while (nanosleep(&ts, &ts) == -1 && errno == EINTR) {
    }
}

static void report(struct text *t, const char *status, const char *kind, const char *extra) {
    text_put(t, "{\"nurt\":\"1.0\",\"report\":{\"nurt\":\"1.0\",\"status\":\"");
    text_put(t, status);
    text_put(t, "\",\"kind\":\"");
    text_put(t, kind);
    text_put(t, "\"");
    text_put(t, extra);
    text_put(t, "}");
}

static void rendered(struct text *t, const char *body) {
    text_put(t, ",\"rendered\":{\"format\":\"text\",\"text\":");
    text_json_string(t, body);
    text_put(t, "}}");
}

static const char *SYNTAX_ERROR =
    ",\"error\":{\"code\":\"E_SYNTAX\",\"message\":\"unexpected end of program\","
    "\"stage\":\"parse\",\"at\":{\"line\":1,\"column\":5},\"fix\":[\"complete the program\"]}";

/* Second document of nurt_parse and nurt_check after a language error, as the library writes it. */
static const char *const LANGUAGE_ERROR_DOCUMENT =
    "{\"diagnostics\":[{\"severity\":\"error\",\"message\":\"unexpected end of program\","
    "\"at\":{\"line\":1,\"column\":5}}],\"error\":{\"code\":\"E_SYNTAX\","
    "\"message\":\"unexpected end of program\",\"stage\":\"parse\"}}";

/* Builds the response of a program; returns a status other than NURT_OK for call failures. */
static nurt_status respond(const char *request, const char *program, struct text *out) {
    if (starts_with(program, "status ")) {
        return fail((nurt_status)atoi(program + 7), "failure requested by the program");
    }
    if (strcmp(program, "panic") == 0) return fail(NURT_E_INTERNAL, "panic caught at the boundary");
    if (strcmp(program, "fail") == 0) {
        report(out, "failed", "parse", SYNTAX_ERROR);
        rendered(out, "error E_SYNTAX: unexpected end of program");
    } else if (strcmp(program, "page") == 0) {
        report(out, "ok", "mock", ",\"next_cursor\":\"page-2\"");
        rendered(out, "page 1");
    } else if (strcmp(program, "options") == 0) {
        char *options = json_object_field(request, "options");
        report(out, "ok", "mock", "");
        rendered(out, options ? options : "{}");
        free(options);
    } else {
        report(out, "ok", "mock", "");
        rendered(out, program);
    }
    return NURT_OK;
}

static nurt_status check_engine(nurt_engine *e) {
    if (!e) return fail(NURT_E_ARG, "engine is null");
    if (e->pid != getpid()) return fail(NURT_E_INTERNAL, "engine used after fork");
    return NURT_OK;
}

static int variant_valid(nurt_variant v) { return v >= NURT_VARIANT_A && v <= NURT_VARIANT_C; }

static uint32_t reported_abi(void) {
    const char *value = getenv("NURT_MOCK_ABI");
    unsigned major = NURT_ABI_MAJOR, minor = NURT_ABI_MINOR;
    if (value) sscanf(value, "%u.%u", &major, &minor);
    return ((uint32_t)major << 16) | (uint32_t)minor;
}

NURT_API uint32_t NURT_CALL nurt_abi_version(void) { return reported_abi(); }

NURT_API nurt_status NURT_CALL nurt_abi_check(uint32_t header_version) {
    FORCED("nurt_abi_check");
    uint32_t library = reported_abi();
    if ((library >> 16) != (header_version >> 16) || (library & 0xFFFF) < (header_version & 0xFFFF)) {
        return fail(NURT_E_ABI, "incompatible interface version");
    }
    return NURT_OK;
}

NURT_API const char *NURT_CALL nurt_version(void) { return "0.1.0"; }

NURT_API nurt_status NURT_CALL nurt_info(nurt_buf *out_json) {
    FORCED("nurt_info");
    if (!out_json) return fail(NURT_E_ARG, "output buffer is null");
    put_string(out_json, "{\"version\":\"0.1.0\",\"abi\":\"1.0\",\"implementation\":\"mock\"}");
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_engine_open(const char *config_json, nurt_engine **out) {
    FORCED("nurt_engine_open");
    if (!out) return fail(NURT_E_ARG, "output handle is null");
    if (config_json) {
        const char *p = config_json;
        while (*p == ' ' || *p == '\n') ++p;
        if (*p != '{') return fail(NURT_E_ARG, "configuration is not a JSON object");
    }
    nurt_engine *e = calloc(1, sizeof *e);
    if (!e) return fail(NURT_E_INTERNAL, "out of memory");
    e->pid = getpid();
    atomic_fetch_add(&live_engines, 1);
    *out = e;
    emit_log(NURT_LOG_INFO, "engine opened");
    return NURT_OK;
}

NURT_API void NURT_CALL nurt_engine_close(nurt_engine *e) {
    if (!e) return;
    atomic_fetch_sub(&live_engines, 1);
    free(e);
}

NURT_API nurt_status NURT_CALL nurt_engine_config_get(nurt_engine *e, nurt_buf *out_json) {
    FORCED("nurt_engine_config_get");
    nurt_status s = check_engine(e);
    if (s) return s;
    put_string(out_json, "{\"nurt\":\"1.0\"}");
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_parse(nurt_engine *e, const char *program, nurt_variant v,
                                          nurt_buf *out_ast_json, nurt_buf *out_diag_json) {
    FORCED("nurt_parse");
    nurt_status s = check_engine(e);
    if (s) return s;
    if (!program || !variant_valid(v)) return fail(NURT_E_ARG, "invalid program or variant");
    if (strcmp(program, "fail") == 0) {
        put_string(out_ast_json, "null");
        put_string(out_diag_json, LANGUAGE_ERROR_DOCUMENT);
        return NURT_OK;
    }
    struct text ast = {0};
    text_put(&ast, "{\"nurt\":\"1.0\",\"program\":");
    text_json_string(&ast, program);
    text_put(&ast, "}");
    put_text(out_ast_json, &ast);
    put_string(out_diag_json, "{\"diagnostics\":[]}");
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_print(nurt_engine *e, const char *ast_json, nurt_variant v,
                                          nurt_buf *out_text) {
    FORCED("nurt_print");
    nurt_status s = check_engine(e);
    if (s) return s;
    char *program = json_string_field(ast_json, "program");
    if (!program || !variant_valid(v)) {
        free(program);
        return fail(NURT_E_ARG, "invalid syntax tree or variant");
    }
    put_string(out_text, program);
    free(program);
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_correct(nurt_engine *e, const char *program, nurt_variant v,
                                            nurt_buf *out_result_json) {
    FORCED("nurt_correct");
    nurt_status s = check_engine(e);
    if (s) return s;
    if (!program || !variant_valid(v)) return fail(NURT_E_ARG, "invalid program or variant");
    struct text result = {0};
    text_put(&result, "{\"program\":");
    text_json_string(&result, program);
    text_put(&result, ",\"corrections\":[]}");
    put_text(out_result_json, &result);
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_check(nurt_engine *e, const char *request_json,
                                          nurt_buf *out_bound_json, nurt_buf *out_diag_json) {
    FORCED("nurt_check");
    nurt_status s = check_engine(e);
    if (s) return s;
    char *program = json_string_field(request_json, "program");
    if (!program) return fail(NURT_E_ARG, "request has no program");
    struct text bound = {0};
    if (strcmp(program, "fail") == 0) {
        text_put(&bound, "null");
        put_string(out_diag_json, LANGUAGE_ERROR_DOCUMENT);
    } else {
        text_put(&bound, "{\"bound\":{\"nurt\":\"1.0\",\"program\":");
        text_json_string(&bound, program);
        text_put(&bound, "},\"effects\":[]}");
        put_string(out_diag_json, "{\"diagnostics\":[]}");
    }
    put_text(out_bound_json, &bound);
    free(program);
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_run_json(nurt_engine *e, const char *request_json,
                                             nurt_buf *out_response_json) {
    FORCED("nurt_run_json");
    nurt_status s = check_engine(e);
    if (s) return s;
    if (!out_response_json) return fail(NURT_E_ARG, "output buffer is null");
    char *program = json_string_field(request_json, "program");
    if (!program) return fail(NURT_E_ARG, "request has no program");
    if (starts_with(program, "sleep ")) sleep_ms(atol(program + 6));
    struct text response = {0};
    s = respond(request_json, program, &response);
    free(program);
    if (s) {
        free(response.data);
        return s;
    }
    put_text(out_response_json, &response);
    return NURT_OK;
}

static void emit_event(nurt_exec *x, const char *type, long seq) {
    if (!x->cb) return;
    char event[160];
    snprintf(event, sizeof event,
             "{\"t\":\"%s\",\"seq\":%ld,\"time\":\"2026-01-01T00:00:00Z\",\"progress\":%ld}", type, seq,
             seq);
    x->cb(x->user, event);
}

static int is_cancelled(nurt_exec *x) {
    pthread_mutex_lock(&x->lock);
    int cancelled = x->cancelled;
    pthread_mutex_unlock(&x->lock);
    return cancelled;
}

static void *exec_main(void *arg) {
    nurt_exec *x = arg;
    long seq = 0;
    if (starts_with(x->program, "events ")) {
        long count = atol(x->program + 7);
        for (long i = 0; i < count; ++i) emit_event(x, "job.progress", ++seq);
    }
    if (starts_with(x->program, "sleep ")) {
        for (long left = atol(x->program + 6); left > 0 && !is_cancelled(x); left -= SLICE_MS) {
            sleep_ms(left < SLICE_MS ? left : SLICE_MS);
        }
    }
    struct text response = {0};
    nurt_status status;
    if (is_cancelled(x)) {
        report(&response, "failed", "mock",
               ",\"error\":{\"code\":\"E_CANCELLED\",\"message\":\"cancelled\"}");
        text_put(&response, "}");
        status = NURT_E_CANCELLED;
    } else {
        status = respond("", x->program, &response);
    }
    emit_event(x, "exec.finished", ++seq);
    pthread_mutex_lock(&x->lock);
    x->response = response.data;
    x->status = status;
    x->finished = 1;
    pthread_cond_broadcast(&x->done);
    pthread_mutex_unlock(&x->lock);
    return NULL;
}

NURT_API nurt_status NURT_CALL nurt_exec_start(nurt_engine *e, const char *request_json,
                                               nurt_event_cb cb, void *user, nurt_exec **out) {
    FORCED("nurt_exec_start");
    nurt_status s = check_engine(e);
    if (s) return s;
    if (!out) return fail(NURT_E_ARG, "output handle is null");
    char *program = json_string_field(request_json, "program");
    if (!program) return fail(NURT_E_ARG, "request has no program");
    nurt_exec *x = calloc(1, sizeof *x);
    if (!x) abort();
    pthread_mutex_init(&x->lock, NULL);
    pthread_cond_init(&x->done, NULL);
    x->cb = cb;
    x->user = user;
    x->program = program;
    if (pthread_create(&x->thread, NULL, exec_main, x) != 0) {
        free(program);
        free(x);
        return fail(NURT_E_INTERNAL, "cannot start a thread");
    }
    atomic_fetch_add(&live_executions, 1);
    *out = x;
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_exec_wait(nurt_exec *x, uint32_t timeout_ms,
                                              nurt_buf *out_response_json) {
    FORCED("nurt_exec_wait");
    if (!x) return fail(NURT_E_ARG, "execution is null");
    pthread_mutex_lock(&x->lock);
    if (timeout_ms == NURT_WAIT_FOREVER) {
        while (!x->finished) pthread_cond_wait(&x->done, &x->lock);
    } else {
        struct timespec deadline;
        clock_gettime(CLOCK_REALTIME, &deadline);
        deadline.tv_sec += timeout_ms / 1000;
        deadline.tv_nsec += (long)(timeout_ms % 1000) * 1000000L;
        if (deadline.tv_nsec >= 1000000000L) {
            deadline.tv_sec += 1;
            deadline.tv_nsec -= 1000000000L;
        }
        while (!x->finished) {
            if (pthread_cond_timedwait(&x->done, &x->lock, &deadline) == ETIMEDOUT) break;
        }
    }
    int finished = x->finished;
    nurt_status status = x->status;
    if (finished && (status == NURT_OK || status == NURT_E_CANCELLED) && x->response) {
        put_string(out_response_json, x->response);
    }
    pthread_mutex_unlock(&x->lock);
    if (!finished) return NURT_E_TIMEOUT;
    if (status != NURT_OK && status != NURT_E_CANCELLED) return fail(status, "execution failed");
    return status;
}

NURT_API nurt_status NURT_CALL nurt_exec_cancel(nurt_exec *x) {
    FORCED("nurt_exec_cancel");
    if (!x) return fail(NURT_E_ARG, "execution is null");
    pthread_mutex_lock(&x->lock);
    x->cancelled = 1;
    pthread_mutex_unlock(&x->lock);
    return NURT_OK;
}

NURT_API void NURT_CALL nurt_exec_free(nurt_exec *x) {
    if (!x) return;
    nurt_exec_cancel(x);
    pthread_join(x->thread, NULL);
    pthread_mutex_destroy(&x->lock);
    pthread_cond_destroy(&x->done);
    free(x->program);
    free(x->response);
    free(x);
    atomic_fetch_sub(&live_executions, 1);
}

NURT_API nurt_status NURT_CALL nurt_page(nurt_engine *e, const char *cursor, uint32_t limit,
                                         nurt_buf *out_response_json) {
    FORCED("nurt_page");
    (void)limit;
    nurt_status s = check_engine(e);
    if (s) return s;
    if (!cursor) return fail(NURT_E_ARG, "cursor is null");
    struct text response = {0};
    if (strcmp(cursor, "page-2") == 0) {
        report(&response, "ok", "mock", "");
        rendered(&response, "page 2");
    } else {
        report(&response, "failed", "page",
               ",\"error\":{\"code\":\"E_ARG\",\"message\":\"the cursor has expired\"}");
        text_put(&response, "}");
    }
    put_text(out_response_json, &response);
    return NURT_OK;
}

static nurt_status reply_json(nurt_engine *e, nurt_buf *out, const char *json) {
    nurt_status s = check_engine(e);
    if (s) return s;
    if (!out) return fail(NURT_E_ARG, "output buffer is null");
    put_string(out, json);
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_catalog(nurt_engine *e, const char *what_json, nurt_buf *out_json) {
    FORCED("nurt_catalog");
    (void)what_json;
    return reply_json(e, out_json, "{\"catalog_rev\":\"mock\",\"groups\":[]}");
}

NURT_API nurt_status NURT_CALL nurt_describe(nurt_engine *e, const char *what_json, nurt_buf *out_text) {
    FORCED("nurt_describe");
    nurt_status s = check_engine(e);
    if (s) return s;
    struct text text = {0};
    text_put(&text, "description ");
    text_put(&text, what_json ? what_json : "{}");
    put_text(out_text, &text);
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_help(nurt_engine *e, const char *topic, nurt_buf *out_text) {
    FORCED("nurt_help");
    nurt_status s = check_engine(e);
    if (s) return s;
    struct text text = {0};
    text_put(&text, "help ");
    text_put(&text, topic ? topic : "index");
    put_text(out_text, &text);
    return NURT_OK;
}

NURT_API nurt_status NURT_CALL nurt_schema(const char *which, nurt_buf *out_json) {
    FORCED("nurt_schema");
    static const char *const names[] = {"ast", "bound", "report", "request", "config", "catalog", "value", "tool"};
    if (!which || !out_json) return fail(NURT_E_ARG, "schema name is null");
    for (size_t i = 0; i < sizeof names / sizeof names[0]; ++i) {
        if (strcmp(which, names[i]) == 0) {
            char json[64];
            snprintf(json, sizeof json, "{\"title\":\"nurt-%s/1\"}", which);
            put_string(out_json, json);
            return NURT_OK;
        }
    }
    return fail(NURT_E_ARG, "unknown schema name");
}

NURT_API nurt_status NURT_CALL nurt_history(nurt_engine *e, const char *query_json, nurt_buf *out_json) {
    FORCED("nurt_history");
    if (query_json) {
        struct text reply = {0};
        text_put(&reply, "{\"entries\":[],\"received\":");
        text_json_string(&reply, query_json);
        text_put(&reply, "}");
        nurt_status s = reply_json(e, out_json, reply.data);
        free(reply.data);
        return s;
    }
    return reply_json(e, out_json, "{\"entries\":[]}");
}

NURT_API nurt_status NURT_CALL nurt_space(nurt_engine *e, const char *volume, nurt_buf *out_json) {
    FORCED("nurt_space");
    (void)volume;
    return reply_json(e, out_json, "{\"volumes\":[]}");
}

NURT_API nurt_status NURT_CALL nurt_session_end(nurt_engine *e, const char *session) {
    FORCED("nurt_session_end");
    nurt_status s = check_engine(e);
    if (s) return s;
    return session ? NURT_OK : fail(NURT_E_ARG, "session is null");
}

NURT_API nurt_status NURT_CALL nurt_ext_load(nurt_engine *e, const char *manifest_path) {
    FORCED("nurt_ext_load");
    nurt_status s = check_engine(e);
    if (s) return s;
    return manifest_path ? fail(NURT_E_IO, "the mock loads no extensions") : fail(NURT_E_ARG, "path is null");
}

NURT_API nurt_status NURT_CALL nurt_ext_list(nurt_engine *e, nurt_buf *out_json) {
    FORCED("nurt_ext_list");
    return reply_json(e, out_json, "{\"extensions\":[]}");
}

NURT_API void NURT_CALL nurt_buf_free(nurt_buf *b) {
    if (!b || !b->ptr) return;
    free(b->ptr);
    b->ptr = NULL;
    b->len = 0;
    b->cap = 0;
    atomic_fetch_sub(&live_buffers, 1);
}

NURT_API nurt_status NURT_CALL nurt_last_error(nurt_engine *e, nurt_buf *out_json) {
    FORCED("nurt_last_error");
    (void)e;
    if (!out_json) return fail(NURT_E_ARG, "output buffer is null");
    if (last_error_set) put_string(out_json, last_error);
    return NURT_OK;
}

NURT_API void NURT_CALL nurt_set_log_callback(nurt_log_cb cb, void *user) {
    pthread_mutex_lock(&log_lock);
    log_receiver = cb;
    log_user = user;
    pthread_mutex_unlock(&log_lock);
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
