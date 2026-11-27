// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           crates/nurt-ffi/tests/abi/c/pipeline_test.c
// Responsibility:   Source file crates/nurt-ffi/tests/abi/c/pipeline_test.c.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

/*
 * Pipeline test of libnurt in C: programs of the form `find |> filter |>
 * count` and `find |> remove`, with and without `preview`, are run through
 * nurt_run_json on an engine over a virtual file system (cargo feature
 * `e2e`, environment variable NURT_E2E_ENGINE). The engine holds the files
 * /data/a.log (10 bytes), /data/b.log (200) and /data/c.log (300). The
 * program prints "ok" or one line per failed check and exits with status 1.
 */

#include "nurt.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define LIST "find /data"
#define COUNT_LARGE "find /data |> filter size > 150 |> count"
#define REMOVE_ALL "find /data |> remove"
#define REMOVE_LARGE "find /data |> filter size > 150 |> remove"

static int failures = 0;

#define CHECK(condition, what, response)                                       \
    do {                                                                       \
        if (!(condition)) {                                                    \
            fprintf(stderr, "check failed at line %d: %s\n%s\n", __LINE__,     \
                    what, (response) != NULL ? (response) : "");               \
            failures++;                                                        \
        }                                                                      \
    } while (0)

static nurt_engine *open_engine(void) {
    nurt_engine *engine = NULL;
    if (nurt_engine_open(NULL, &engine) != NURT_OK) {
        fprintf(stderr, "cannot open the engine\n");
        exit(2);
    }
    return engine;
}

/* Runs a program; the response is a NUL-terminated string owned by the caller. */
static char *run(nurt_engine *e, const char *program, int preview) {
    char request[512];
    nurt_buf out = {0};
    char *response;
    snprintf(request, sizeof request,
             "{\"nurt\":\"1.0\",\"program\":\"%s\",\"options\":{\"preview\":%s,\"verbosity\":\"detailed\"}}",
             program, preview ? "true" : "false");
    if (nurt_run_json(e, request, &out) != NURT_OK || out.ptr == NULL) {
        fprintf(stderr, "nurt_run_json failed for %s\n", program);
        exit(2);
    }
    response = malloc(out.len + 1);
    if (response == NULL) {
        exit(2);
    }
    memcpy(response, out.ptr, out.len);
    response[out.len] = '\0';
    nurt_buf_free(&out);
    return response;
}

static int occurrences(const char *text, const char *needle) {
    int count = 0;
    const char *at = text;
    while ((at = strstr(at, needle)) != NULL) {
        count++;
        at += strlen(needle);
    }
    return count;
}

static int has(const char *text, const char *needle) {
    return strstr(text, needle) != NULL;
}

static void test_count(void) {
    nurt_engine *e = open_engine();
    char *response = run(e, COUNT_LARGE, 0);
    CHECK(has(response, "\"status\":\"ok\""), "count status", response);
    CHECK(has(response, "\"value\":{\"t\":\"int\",\"v\":2}"), "two files are larger than 150 bytes", response);
    CHECK(!has(response, "\"changes\""), "counting changes nothing", response);
    free(response);
    nurt_engine_close(e);
}

static void test_remove_with_preview(void) {
    nurt_engine *e = open_engine();
    char *response = run(e, REMOVE_ALL, 1);
    CHECK(has(response, "\"status\":\"planned\""), "preview status", response);
    CHECK(occurrences(response, "\"action\":\"delete\"") == 3, "preview plans three deletions", response);
    free(response);
    response = run(e, LIST, 0);
    CHECK(occurrences(response, ".log\"") == 3, "preview keeps the files", response);
    free(response);
    nurt_engine_close(e);
}

static void test_remove_without_preview(void) {
    nurt_engine *e = open_engine();
    char *response = run(e, REMOVE_LARGE, 0);
    CHECK(has(response, "\"status\":\"ok\""), "remove status", response);
    CHECK(occurrences(response, "\"action\":\"delete\"") == 2, "two deletions", response);
    free(response);
    response = run(e, LIST, 0);
    CHECK(has(response, "a.log") && !has(response, "b.log") && !has(response, "c.log"),
          "only the small file remains", response);
    free(response);
    response = run(e, REMOVE_ALL, 0);
    CHECK(occurrences(response, "\"action\":\"delete\"") == 1, "one file left to delete", response);
    free(response);
    response = run(e, COUNT_LARGE, 0);
    CHECK(has(response, "\"value\":{\"t\":\"int\",\"v\":0}"), "nothing is left to count", response);
    free(response);
    nurt_engine_close(e);
}

int main(void) {
    test_count();
    test_remove_with_preview();
    test_remove_without_preview();
    if (failures != 0) {
        fprintf(stderr, "%d checks failed\n", failures);
        return 1;
    }
    printf("ok\n");
    return 0;
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
