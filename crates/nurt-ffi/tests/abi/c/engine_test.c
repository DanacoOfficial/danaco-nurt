// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
/*
 * Interface test of libnurt in C, run against the engine of nurt-core in a
 * build without the cargo feature `stub`. The engine of this crate has no
 * language front end, so programs end with language errors in the JSON
 * results. Every status returned by the engine is checked against the
 * status table of nurt.h. The state directory of the engine is placed under
 * $HOME. The program prints one line per failure and exits with status 1
 * when a check fails.
 */

#include "nurt.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_EVENTS 16
#define PATH_SIZE 4096
#define BIT(status) (1u << (unsigned)(status))
#define USUAL (BIT(NURT_OK) | BIT(NURT_E_ARG) | BIT(NURT_E_INTERNAL))

static const char *const READ = "{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\"\"}";
static const char *const TWO_INPUTS = "{\"nurt\":\"1.0\",\"program\":\"find \\\"/srv/app\\\"\",\"cursor\":\"c_91fe\"}";
static const char *const PROGRAM = "find \"/srv/app\"";
static const char *const AST = "{\"nurt\":\"1.0\",\"program\":{\"statements\":[]}}";

static int failures = 0;

#define CHECK(condition, what)                                              \
    do {                                                                    \
        if (!(condition)) {                                                 \
            fprintf(stderr, "check failed at line %d: %s\n", __LINE__, what); \
            failures++;                                                     \
        }                                                                   \
    } while (0)

/* Rows of the status table of nurt.h for the functions that take an engine. */
static const struct {
    const char *function;
    unsigned allowed;
} MATRIX[] = {
    {"nurt_engine_config_get", USUAL},
    {"nurt_parse", USUAL},
    {"nurt_print", USUAL},
    {"nurt_correct", USUAL},
    {"nurt_check", USUAL},
    {"nurt_run_json", USUAL},
    {"nurt_exec_start", USUAL},
    {"nurt_page", USUAL},
    {"nurt_catalog", USUAL},
    {"nurt_describe", USUAL},
    {"nurt_help", USUAL},
    {"nurt_history", USUAL},
    {"nurt_space", USUAL},
    {"nurt_session_end", USUAL | BIT(NURT_E_IO)},
    {"nurt_ext_load", USUAL | BIT(NURT_E_ABI) | BIT(NURT_E_IO)},
    {"nurt_ext_list", USUAL},
};

static nurt_engine *open_engine(void) {
    nurt_engine *engine = NULL;
    if (nurt_engine_open(NULL, &engine) != NURT_OK) {
        fprintf(stderr, "cannot open the engine\n");
        exit(2);
    }
    return engine;
}

static int contains(const nurt_buf *buf, const char *text) {
    return buf->ptr != NULL && strstr((const char *)buf->ptr, text) != NULL;
}

static int last_error_contains(const char *text) {
    nurt_buf record = {0};
    int found;
    nurt_last_error(NULL, &record);
    found = contains(&record, text);
    nurt_buf_free(&record);
    return found;
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
        status = nurt_correct(e, PROGRAM, NURT_VARIANT_A, &a);
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

static void test_matrix(void) {
    nurt_engine *e = open_engine();
    nurt_engine *other = NULL;
    nurt_exec *x = NULL;
    nurt_buf out = {0};
    size_t i;
    for (i = 0; i < sizeof MATRIX / sizeof MATRIX[0]; i++) {
        nurt_status status = engine_call(MATRIX[i].function, e);
        if ((MATRIX[i].allowed & BIT(status)) == 0) {
            fprintf(stderr, "%s returned %d\n", MATRIX[i].function, (int)status);
            failures++;
        }
        CHECK(engine_call(MATRIX[i].function, NULL) == NURT_E_ARG, MATRIX[i].function);
    }
    CHECK(nurt_abi_check(NURT_ABI_VERSION) == NURT_OK, "nurt_abi_check");
    CHECK(nurt_abi_check(NURT_ABI_VERSION + (1u << 16)) == NURT_E_ABI, "nurt_abi_check ABI");
    CHECK(nurt_info(&out) == NURT_OK, "nurt_info");
    CHECK(contains(&out, "\"catalog_rev\":\"sha256:"), "catalog revision");
    nurt_buf_free(&out);
    CHECK(nurt_info(NULL) == NURT_E_ARG, "nurt_info with NULL");
    CHECK(nurt_engine_open(NULL, &other) == NURT_OK, "nurt_engine_open");
    nurt_engine_close(other);
    CHECK(nurt_engine_open(NULL, NULL) == NURT_E_ARG, "nurt_engine_open with NULL");
    CHECK(nurt_schema("ast", &out) == NURT_OK, "nurt_schema");
    nurt_buf_free(&out);
    CHECK(nurt_schema("unknown", &out) == NURT_E_ARG, "nurt_schema with an unknown name");
    CHECK(nurt_last_error(NULL, &out) == NURT_OK, "nurt_last_error");
    nurt_buf_free(&out);
    CHECK(nurt_last_error(NULL, NULL) == NURT_E_ARG, "nurt_last_error with NULL");
    CHECK(nurt_exec_start(e, READ, NULL, NULL, &x) == NURT_OK, "nurt_exec_start");
    CHECK(nurt_exec_cancel(x) == NURT_OK, "nurt_exec_cancel");
    CHECK((BIT(nurt_exec_wait(x, NURT_WAIT_FOREVER, &out)) & (BIT(NURT_OK) | BIT(NURT_E_CANCELLED))) != 0,
          "nurt_exec_wait");
    nurt_buf_free(&out);
    nurt_exec_free(x);
    CHECK(nurt_exec_wait(NULL, 0, &out) == NURT_E_ARG, "nurt_exec_wait with NULL");
    CHECK(nurt_exec_cancel(NULL) == NURT_E_ARG, "nurt_exec_cancel with NULL");
    nurt_engine_close(e);
}

static void test_run_json(void) {
    nurt_engine *e = open_engine();
    nurt_buf out = {0};
    CHECK(nurt_run_json(e, READ, &out) == NURT_OK, "read request");
    CHECK(contains(&out, "\"E_UNSUPPORTED\""), "language error of the read request");
    nurt_buf_free(&out);
    CHECK(nurt_run_json(e, TWO_INPUTS, &out) == NURT_OK, "envelope rule keeps NURT_OK");
    CHECK(contains(&out, "\"E_ARG\""), "language error E_ARG");
    nurt_buf_free(&out);
    CHECK(nurt_run_json(e, "not json", &out) == NURT_E_ARG, "invalid JSON");
    CHECK(out.ptr == NULL && out.len == 0, "failing call leaves the buffer empty");
    CHECK(last_error_contains("\"NURT_E_ARG\""), "last error record");
    CHECK(nurt_print(e, AST, NURT_VARIANT_A, &out) == NURT_E_ARG, "print without a front end");
    CHECK(last_error_contains("\"E_UNSUPPORTED\""), "language error in the last error");
    {
        nurt_buf diag = {0};
        CHECK(nurt_parse(e, PROGRAM, NURT_VARIANT_A, &out, &diag) == NURT_OK, "parse keeps NURT_OK");
        CHECK(out.len == 4 && strcmp((const char *)out.ptr, "null") == 0, "no syntax tree");
        CHECK(contains(&diag, "\"E_UNSUPPORTED\""), "language error in the diagnostics");
        nurt_buf_free(&diag);
    }
    nurt_buf_free(&out);
    nurt_engine_close(e);
}

struct events {
    int count;
    int with_request_id;
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
    events->with_request_id += strstr(event_json, "\"request_id\":\"") != NULL;
    events->count++;
}

static void test_events(void) {
    nurt_engine *e = open_engine();
    nurt_exec *x = NULL;
    nurt_buf out = {0};
    struct events events = {0};
    CHECK(nurt_exec_start(e, READ, collect, &events, &x) == NURT_OK, "start with events");
    CHECK(nurt_exec_wait(x, NURT_WAIT_FOREVER, &out) == NURT_OK, "wait for completion");
    nurt_buf_free(&out);
    CHECK(nurt_exec_wait(x, 0, &out) == NURT_OK, "same result after completion");
    nurt_buf_free(&out);
    nurt_exec_free(x);
    CHECK(events.count == 2, "two events");
    CHECK(strcmp(events.types[0], "exec.started") == 0, "first event is exec.started");
    CHECK(strcmp(events.types[1], "exec.finished") == 0, "last event is exec.finished");
    CHECK(events.seq[0] == 1 && events.seq[1] == 2, "seq increases from 1");
    CHECK(events.with_request_id == 2, "events carry the request identifier");
    nurt_engine_close(e);
}

static void test_io(void) {
    char blocker[PATH_SIZE];
    char config[PATH_SIZE + 64];
    const char *home = getenv("HOME");
    nurt_engine *e = NULL;
    FILE *file;
    if (home == NULL) {
        fprintf(stderr, "HOME is not set\n");
        exit(2);
    }
    snprintf(blocker, sizeof blocker, "%s/blocker", home);
    file = fopen(blocker, "w");
    if (file == NULL) {
        fprintf(stderr, "cannot create %s\n", blocker);
        exit(2);
    }
    fclose(file);
    snprintf(config, sizeof config, "{\"state\":{\"dir\":\"%s/state\"}}", blocker);
    CHECK(nurt_engine_open(config, &e) == NURT_E_IO, "state directory under a file");
    CHECK(e == NULL, "no engine after a failure");
    CHECK(nurt_engine_open("{\"state\":{\"dir\":\"state\"}}", &e) == NURT_E_ARG, "relative state directory");
}

int main(void) {
    CHECK(nurt_abi_version() == NURT_ABI_VERSION, "interface version");
    CHECK(strlen(nurt_version()) > 0, "library version");
    test_matrix();
    test_run_json();
    test_events();
    test_io();
    if (failures > 0) {
        fprintf(stderr, "%d checks failed\n", failures);
        return 1;
    }
    printf("ok\n");
    return 0;
}
