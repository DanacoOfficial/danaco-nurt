// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
/*
 * Interface test of libnurt in C, run against the engine of the cargo
 * feature `stub`. Every cell of the status table of nurt.h is exercised;
 * the program prints one line per failure and exits with status 1 when a
 * check fails.
 */

#include "nurt.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define PANIC_VAR "NURT_STUB_PANIC"
#define CASES_DIR_VAR "NURT_STUB_DIR"
#define MAX_EVENTS 16

static const char *const READ = "{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\" glob: \\\"*.go\\\"\"}";
static const char *const SYNTAX_ERROR = "{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\" glob: [\\\"*.go\\\"\"}";
static const char *const TWO_INPUTS = "{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\"\",\"cursor\":\"c_91fe\"}";
static const char *const EVENTS = "{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\" glob: \\\"*.rs\\\"\"}";
static const char *const SLOW = "{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/archive\\\"\"}";
static const char *const PROGRAM = "find \"/srv/app\"";
static const char *const MISSPELT = "fnd \"/srv/app\"";
static const char *const AST =
    "{\"nurt\":\"1.0\",\"program\":{\"statements\":[{\"t\":\"pipeline\",\"head\":{\"t\":\"stage\",\"name\":\"find\",\"args\":[{\"value\":{\"t\":\"text\",\"v\":\"/srv/app\"}}]},\"stages\":[]}]}}";

static int failures = 0;

#define CHECK(condition, what)                                              \
    do {                                                                    \
        if (!(condition)) {                                                 \
            fprintf(stderr, "check failed at line %d: %s\n", __LINE__, what); \
            failures++;                                                     \
        }                                                                   \
    } while (0)

static nurt_engine *open_engine(void) {
    nurt_engine *engine = NULL;
    if (nurt_engine_open(NULL, &engine) != NURT_OK) {
        fprintf(stderr, "cannot open the stub engine\n");
        exit(2);
    }
    return engine;
}

static int contains(const nurt_buf *buf, const char *text) {
    return buf->ptr != NULL && strstr((const char *)buf->ptr, text) != NULL;
}

/* Calls a function that takes an engine with valid other arguments. */
static nurt_status engine_call(const char *function, nurt_engine *e) {
    nurt_buf a = {0}, b = {0};
    nurt_status status;
    if (strcmp(function, "nurt_engine_config_get") == 0) {
        status = nurt_engine_config_get(e, &a);
    } else if (strcmp(function, "nurt_parse") == 0) {
        status = nurt_parse(e, PROGRAM, NURT_VARIANT_A, &a, &b);
    } else if (strcmp(function, "nurt_print") == 0) {
        status = nurt_print(e, AST, NURT_VARIANT_A, &a);
    } else if (strcmp(function, "nurt_correct") == 0) {
        status = nurt_correct(e, MISSPELT, NURT_VARIANT_A, &a);
    } else if (strcmp(function, "nurt_check") == 0) {
        status = nurt_check(e, READ, &a, &b);
    } else if (strcmp(function, "nurt_run_json") == 0) {
        status = nurt_run_json(e, READ, &a);
    } else if (strcmp(function, "nurt_exec_start") == 0) {
        nurt_exec *x = NULL;
        status = nurt_exec_start(e, READ, NULL, NULL, &x);
        nurt_exec_free(x);
    } else if (strcmp(function, "nurt_page") == 0) {
        status = nurt_page(e, "c_91fe", 0, &a);
    } else if (strcmp(function, "nurt_catalog") == 0) {
        status = nurt_catalog(e, NULL, &a);
    } else if (strcmp(function, "nurt_describe") == 0) {
        status = nurt_describe(e, NULL, &a);
    } else if (strcmp(function, "nurt_help") == 0) {
        status = nurt_help(e, NULL, &a);
    } else if (strcmp(function, "nurt_history") == 0) {
        status = nurt_history(e, NULL, &a);
    } else if (strcmp(function, "nurt_space") == 0) {
        status = nurt_space(e, NULL, &a);
    } else if (strcmp(function, "nurt_session_end") == 0) {
        status = nurt_session_end(e, "s-1");
    } else if (strcmp(function, "nurt_ext_load") == 0) {
        status = nurt_ext_load(e, "/opt/nurt/ext/sample/nurt-ext.toml");
    } else if (strcmp(function, "nurt_ext_list") == 0) {
        status = nurt_ext_list(e, &a);
    } else {
        fprintf(stderr, "no engine scenario for %s\n", function);
        exit(2);
    }
    nurt_buf_free(&a);
    nurt_buf_free(&b);
    return status;
}

static nurt_status with_exec(nurt_engine *e, const char *request, uint32_t timeout, int cancel) {
    nurt_exec *x = NULL;
    nurt_buf out = {0};
    nurt_status status;
    if (nurt_exec_start(e, request, NULL, NULL, &x) != NURT_OK) {
        return NURT_E_INTERNAL;
    }
    if (cancel) {
        nurt_exec_cancel(x);
    }
    status = nurt_exec_wait(x, timeout, &out);
    nurt_buf_free(&out);
    nurt_exec_free(x);
    return status;
}

static nurt_status ok_scenario(const char *function) {
    nurt_buf out = {0};
    nurt_status status;
    nurt_engine *e;
    if (strcmp(function, "nurt_abi_check") == 0) {
        return nurt_abi_check(NURT_ABI_VERSION);
    }
    if (strcmp(function, "nurt_info") == 0) {
        status = nurt_info(&out);
        nurt_buf_free(&out);
        return status;
    }
    if (strcmp(function, "nurt_engine_open") == 0) {
        nurt_engine *other = NULL;
        status = nurt_engine_open(NULL, &other);
        nurt_engine_close(other);
        return status;
    }
    if (strcmp(function, "nurt_schema") == 0) {
        status = nurt_schema("ast", &out);
        nurt_buf_free(&out);
        return status;
    }
    if (strcmp(function, "nurt_last_error") == 0) {
        status = nurt_last_error(NULL, &out);
        nurt_buf_free(&out);
        return status;
    }
    e = open_engine();
    if (strcmp(function, "nurt_exec_wait") == 0) {
        status = with_exec(e, READ, NURT_WAIT_FOREVER, 0);
    } else if (strcmp(function, "nurt_exec_cancel") == 0) {
        nurt_exec *x = NULL;
        nurt_exec_start(e, READ, NULL, NULL, &x);
        status = nurt_exec_cancel(x);
        nurt_exec_free(x);
    } else {
        status = engine_call(function, e);
    }
    nurt_engine_close(e);
    return status;
}

static nurt_status arg_scenario(const char *function) {
    nurt_buf out = {0};
    nurt_status status;
    if (strcmp(function, "nurt_info") == 0) {
        return nurt_info(NULL);
    }
    if (strcmp(function, "nurt_engine_open") == 0) {
        return nurt_engine_open(NULL, NULL);
    }
    if (strcmp(function, "nurt_exec_wait") == 0) {
        return nurt_exec_wait(NULL, 0, &out);
    }
    if (strcmp(function, "nurt_exec_cancel") == 0) {
        return nurt_exec_cancel(NULL);
    }
    if (strcmp(function, "nurt_schema") == 0) {
        status = nurt_schema("unknown", &out);
        nurt_buf_free(&out);
        return status;
    }
    if (strcmp(function, "nurt_last_error") == 0) {
        return nurt_last_error(NULL, NULL);
    }
    return engine_call(function, NULL);
}

static nurt_status other_scenario(const char *function, nurt_status expected) {
    nurt_engine *e = open_engine();
    nurt_status status = NURT_E_INTERNAL;
    if (strcmp(function, "nurt_abi_check") == 0 && expected == NURT_E_ABI) {
        status = nurt_abi_check((uint32_t)(NURT_ABI_MAJOR + 1) << 16);
    } else if (strcmp(function, "nurt_ext_load") == 0 && expected == NURT_E_ABI) {
        status = nurt_ext_load(e, "/opt/nurt/ext/future/nurt-ext.toml");
    } else if (strcmp(function, "nurt_ext_load") == 0 && expected == NURT_E_IO) {
        status = nurt_ext_load(e, "/opt/nurt/ext/missing/nurt-ext.toml");
    } else if (strcmp(function, "nurt_session_end") == 0 && expected == NURT_E_IO) {
        status = nurt_session_end(e, "s-io");
    } else if (strcmp(function, "nurt_engine_open") == 0 && expected == NURT_E_IO) {
        nurt_engine *other = NULL;
        setenv(CASES_DIR_VAR, "/nonexistent/nurt-stub-cases", 1);
        status = nurt_engine_open(NULL, &other);
        unsetenv(CASES_DIR_VAR);
        CHECK(other == NULL, "failed nurt_engine_open leaves out NULL");
    } else if (strcmp(function, "nurt_exec_wait") == 0 && expected == NURT_E_TIMEOUT) {
        status = with_exec(e, SLOW, 0, 0);
    } else if (strcmp(function, "nurt_exec_wait") == 0 && expected == NURT_E_CANCELLED) {
        status = with_exec(e, SLOW, NURT_WAIT_FOREVER, 1);
    } else {
        fprintf(stderr, "no scenario for %s with status %d\n", function, (int)expected);
        exit(2);
    }
    nurt_engine_close(e);
    return status;
}

static nurt_status scenario(const char *function, nurt_status expected) {
    nurt_status status;
    switch (expected) {
    case NURT_OK:
        return ok_scenario(function);
    case NURT_E_ARG:
        return arg_scenario(function);
    case NURT_E_INTERNAL:
        setenv(PANIC_VAR, function, 1);
        status = ok_scenario(function);
        unsetenv(PANIC_VAR);
        return status;
    default:
        return other_scenario(function, expected);
    }
}

struct row {
    const char *function;
    nurt_status statuses[7];
    int count;
};

/* Copy of the table "Possible status codes of each function" of nurt.h. */
static const struct row MATRIX[] = {
    {"nurt_abi_check", {NURT_OK, NURT_E_ABI}, 2},
    {"nurt_info", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_engine_open", {NURT_OK, NURT_E_ARG, NURT_E_IO, NURT_E_INTERNAL}, 4},
    {"nurt_engine_config_get", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_parse", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_print", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_correct", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_check", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_run_json", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_exec_start", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_exec_wait", {NURT_OK, NURT_E_TIMEOUT, NURT_E_CANCELLED, NURT_E_ARG, NURT_E_INTERNAL}, 5},
    {"nurt_exec_cancel", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_page", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_catalog", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_describe", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_help", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_schema", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_history", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_space", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_session_end", {NURT_OK, NURT_E_ARG, NURT_E_IO, NURT_E_INTERNAL}, 4},
    {"nurt_ext_load", {NURT_OK, NURT_E_ARG, NURT_E_ABI, NURT_E_IO, NURT_E_INTERNAL}, 5},
    {"nurt_ext_list", {NURT_OK, NURT_E_ARG, NURT_E_INTERNAL}, 3},
    {"nurt_last_error", {NURT_OK, NURT_E_ARG}, 2},
};

static void test_matrix(void) {
    size_t i;
    int j;
    for (i = 0; i < sizeof MATRIX / sizeof MATRIX[0]; i++) {
        for (j = 0; j < MATRIX[i].count; j++) {
            nurt_status expected = MATRIX[i].statuses[j];
            nurt_status actual = scenario(MATRIX[i].function, expected);
            if (actual != expected) {
                fprintf(stderr, "%s: expected status %d, got %d\n", MATRIX[i].function, (int)expected, (int)actual);
                failures++;
            }
        }
    }
}

static void test_run_json(void) {
    nurt_engine *e = open_engine();
    nurt_buf out = {0};
    CHECK(nurt_run_json(e, READ, &out) == NURT_OK, "read request");
    CHECK(contains(&out, "\"matched\":148"), "read response holds the records");
    CHECK(out.ptr[out.len] == 0, "response is NUL-terminated");
    nurt_buf_free(&out);
    CHECK(nurt_run_json(e, SYNTAX_ERROR, &out) == NURT_OK, "language error keeps NURT_OK");
    CHECK(contains(&out, "\"E_SYNTAX\""), "syntax error in the response");
    nurt_buf_free(&out);
    CHECK(nurt_run_json(e, TWO_INPUTS, &out) == NURT_OK, "envelope rule keeps NURT_OK");
    CHECK(contains(&out, "\"E_ARG\""), "envelope rule reported as E_ARG");
    nurt_buf_free(&out);
    CHECK(nurt_run_json(e, "not json", &out) == NURT_E_ARG, "invalid JSON");
    CHECK(out.ptr == NULL && out.len == 0, "failing call leaves the buffer empty");
    CHECK(nurt_last_error(NULL, &out) == NURT_OK && contains(&out, "\"NURT_E_ARG\""), "last error names the status");
    CHECK(contains(&out, "\"nurt_run_json\""), "last error names the function");
    nurt_buf_free(&out);
    nurt_buf_free(&out);
    nurt_buf_free(NULL);
    CHECK(out.ptr == NULL && out.len == 0 && out.cap == 0, "released buffer is zero");
    nurt_engine_close(e);
}

struct events {
    int count;
    char types[MAX_EVENTS][32];
    long seq[MAX_EVENTS];
};

static void collect(void *user, const char *event_json) {
    struct events *events = user;
    const char *type = strstr(event_json, "\"t\":\"");
    const char *seq = strstr(event_json, "\"seq\":");
    if (events->count >= MAX_EVENTS || type == NULL || seq == NULL) {
        return;
    }
    sscanf(type + 5, "%31[^\"]", events->types[events->count]);
    events->seq[events->count] = strtol(seq + 6, NULL, 10);
    events->count++;
}

static void test_events(void) {
    nurt_engine *e = open_engine();
    nurt_exec *x = NULL;
    nurt_buf out = {0};
    struct events events = {0};
    int i;
    CHECK(nurt_exec_start(e, EVENTS, collect, &events, &x) == NURT_OK, "start with events");
    CHECK(nurt_exec_wait(x, NURT_WAIT_FOREVER, &out) == NURT_OK, "wait for completion");
    nurt_exec_free(x);
    CHECK(events.count == 5, "five events");
    CHECK(strcmp(events.types[0], "exec.started") == 0, "first event is exec.started");
    CHECK(strcmp(events.types[events.count - 1], "exec.finished") == 0, "last event is exec.finished");
    for (i = 0; i < events.count; i++) {
        CHECK(events.seq[i] == i + 1, "seq increases from 1");
    }
    nurt_buf_free(&out);

    memset(&events, 0, sizeof events);
    CHECK(nurt_exec_start(e, SLOW, collect, &events, &x) == NURT_OK, "start a slow execution");
    CHECK(nurt_exec_wait(x, 0, &out) == NURT_E_TIMEOUT && out.ptr == NULL, "zero timeout polls");
    CHECK(nurt_exec_cancel(x) == NURT_OK && nurt_exec_cancel(x) == NURT_OK, "cancel is idempotent");
    CHECK(nurt_exec_wait(x, NURT_WAIT_FOREVER, &out) == NURT_E_CANCELLED, "cancelled execution");
    CHECK(contains(&out, "\"E_CANCELLED\""), "response of the cancelled execution");
    nurt_buf_free(&out);
    CHECK(nurt_exec_wait(x, 0, &out) == NURT_E_CANCELLED, "same result after completion");
    nurt_buf_free(&out);
    nurt_exec_free(x);
    CHECK(strcmp(events.types[events.count - 1], "exec.finished") == 0, "cancelled execution ends with exec.finished");
    nurt_engine_close(e);
}

int main(void) {
    CHECK(nurt_abi_version() == NURT_ABI_VERSION, "interface version");
    CHECK(strlen(nurt_version()) > 0, "library version");
    test_matrix();
    test_run_json();
    test_events();
    if (failures > 0) {
        fprintf(stderr, "%d checks failed\n", failures);
        return 1;
    }
    printf("ok\n");
    return 0;
}
