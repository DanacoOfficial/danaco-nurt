// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
/*
 * Node-API module exposing the functions of libnurt to abi_test.js. Null
 * JavaScript values become NULL pointers, results are returned as objects
 * {status, out} with the output buffers as strings, and the events of an
 * execution are collected for inspection after it completes.
 */

#include <node_api.h>

#include "nurt.h"

#include <pthread.h>
#include <stdlib.h>
#include <string.h>

#define MAX_EVENTS 64
#define MAX_ARGS 4

struct exec_box {
    nurt_exec *x;
    pthread_mutex_t lock;
    char *events[MAX_EVENTS];
    int count;
};

static int is_nullish(napi_env env, napi_value value) {
    napi_valuetype type;
    napi_typeof(env, value, &type);
    return type == napi_null || type == napi_undefined;
}

/* Copies a JavaScript string; null and undefined give NULL. */
static char *string_arg(napi_env env, napi_value value) {
    size_t length = 0;
    char *text;
    if (is_nullish(env, value)) {
        return NULL;
    }
    napi_get_value_string_utf8(env, value, NULL, 0, &length);
    text = malloc(length + 1);
    napi_get_value_string_utf8(env, value, text, length + 1, &length);
    return text;
}

static void *handle_arg(napi_env env, napi_value value) {
    void *handle = NULL;
    if (!is_nullish(env, value)) {
        napi_get_value_external(env, value, &handle);
    }
    return handle;
}

static uint32_t number_arg(napi_env env, napi_value value) {
    uint32_t number = 0;
    napi_get_value_uint32(env, value, &number);
    return number;
}

static int flag_arg(napi_env env, napi_value value) {
    bool flag = false;
    napi_get_value_bool(env, value, &flag);
    return flag;
}

static napi_value number(napi_env env, uint32_t value) {
    napi_value result;
    napi_create_uint32(env, value, &result);
    return result;
}

/* Builds {status, out} and releases the buffers. */
static napi_value result(napi_env env, nurt_status status, nurt_buf *bufs, int count) {
    napi_value object, out, item;
    int i;
    napi_create_object(env, &object);
    napi_set_named_property(env, object, "status", number(env, (uint32_t)status));
    napi_create_array_with_length(env, (size_t)count, &out);
    for (i = 0; i < count; i++) {
        if (bufs[i].ptr == NULL) {
            napi_get_null(env, &item);
        } else {
            napi_create_string_utf8(env, (const char *)bufs[i].ptr, bufs[i].len, &item);
        }
        napi_set_element(env, out, (uint32_t)i, item);
        nurt_buf_free(&bufs[i]);
    }
    napi_set_named_property(env, object, "out", out);
    return object;
}

static napi_value with_handle(napi_env env, nurt_status status, void *handle) {
    napi_value object, value;
    napi_create_object(env, &object);
    napi_set_named_property(env, object, "status", number(env, (uint32_t)status));
    if (handle == NULL) {
        napi_get_null(env, &value);
    } else {
        napi_create_external(env, handle, NULL, NULL, &value);
    }
    napi_set_named_property(env, object, "handle", value);
    return object;
}

static void arguments(napi_env env, napi_callback_info info, napi_value *argv) {
    size_t argc = MAX_ARGS;
    int i;
    for (i = 0; i < MAX_ARGS; i++) {
        napi_get_undefined(env, &argv[i]);
    }
    napi_get_cb_info(env, info, &argc, argv, NULL, NULL);
}

static napi_value abi_version(napi_env env, napi_callback_info info) {
    (void)info;
    return number(env, nurt_abi_version());
}

static napi_value abi_check(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS];
    arguments(env, info, argv);
    return number(env, (uint32_t)nurt_abi_check(number_arg(env, argv[0])));
}

static napi_value version(napi_env env, napi_callback_info info) {
    napi_value value;
    (void)info;
    napi_create_string_utf8(env, nurt_version(), NAPI_AUTO_LENGTH, &value);
    return value;
}

static napi_value lib_info(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS];
    nurt_buf out = {0};
    arguments(env, info, argv);
    return result(env, nurt_info(flag_arg(env, argv[0]) ? NULL : &out), &out, 1);
}

static napi_value last_error(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS];
    nurt_buf out = {0};
    arguments(env, info, argv);
    return result(env, nurt_last_error(NULL, flag_arg(env, argv[0]) ? NULL : &out), &out, 1);
}

static napi_value engine_open(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS];
    nurt_engine *engine = NULL;
    char *config;
    nurt_status status;
    arguments(env, info, argv);
    config = string_arg(env, argv[0]);
    status = nurt_engine_open(config, flag_arg(env, argv[1]) ? NULL : &engine);
    free(config);
    return with_handle(env, status, engine);
}

static napi_value engine_close(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS];
    arguments(env, info, argv);
    nurt_engine_close(handle_arg(env, argv[0]));
    return NULL;
}

/* call(name, engine, text, number): the functions with buffers or without
 * output that take an engine, and nurt_schema. */
static napi_value call(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS];
    nurt_buf out[2] = {{0}, {0}};
    char *name, *text;
    nurt_engine *e;
    uint32_t value;
    nurt_status status;
    int count = 1;
    arguments(env, info, argv);
    name = string_arg(env, argv[0]);
    e = handle_arg(env, argv[1]);
    text = string_arg(env, argv[2]);
    value = number_arg(env, argv[3]);
    if (strcmp(name, "nurt_engine_config_get") == 0) {
        status = nurt_engine_config_get(e, &out[0]);
    } else if (strcmp(name, "nurt_parse") == 0) {
        status = nurt_parse(e, text, (nurt_variant)value, &out[0], &out[1]);
        count = 2;
    } else if (strcmp(name, "nurt_print") == 0) {
        status = nurt_print(e, text, (nurt_variant)value, &out[0]);
    } else if (strcmp(name, "nurt_correct") == 0) {
        status = nurt_correct(e, text, (nurt_variant)value, &out[0]);
    } else if (strcmp(name, "nurt_check") == 0) {
        status = nurt_check(e, text, &out[0], &out[1]);
        count = 2;
    } else if (strcmp(name, "nurt_run_json") == 0) {
        status = nurt_run_json(e, text, &out[0]);
    } else if (strcmp(name, "nurt_page") == 0) {
        status = nurt_page(e, text, value, &out[0]);
    } else if (strcmp(name, "nurt_catalog") == 0) {
        status = nurt_catalog(e, text, &out[0]);
    } else if (strcmp(name, "nurt_describe") == 0) {
        status = nurt_describe(e, text, &out[0]);
    } else if (strcmp(name, "nurt_help") == 0) {
        status = nurt_help(e, text, &out[0]);
    } else if (strcmp(name, "nurt_schema") == 0) {
        status = nurt_schema(text, &out[0]);
    } else if (strcmp(name, "nurt_history") == 0) {
        status = nurt_history(e, text, &out[0]);
    } else if (strcmp(name, "nurt_space") == 0) {
        status = nurt_space(e, text, &out[0]);
    } else if (strcmp(name, "nurt_session_end") == 0) {
        status = nurt_session_end(e, text);
        count = 0;
    } else if (strcmp(name, "nurt_ext_load") == 0) {
        status = nurt_ext_load(e, text);
        count = 0;
    } else if (strcmp(name, "nurt_ext_list") == 0) {
        status = nurt_ext_list(e, &out[0]);
    } else {
        free(name);
        free(text);
        napi_throw_error(env, NULL, "unknown function");
        return NULL;
    }
    free(name);
    free(text);
    return result(env, status, out, count);
}

static void collect(void *user, const char *event_json) {
    struct exec_box *box = user;
    pthread_mutex_lock(&box->lock);
    if (box->count < MAX_EVENTS) {
        box->events[box->count++] = strdup(event_json);
    }
    pthread_mutex_unlock(&box->lock);
}

static napi_value exec_start(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS];
    struct exec_box *box = calloc(1, sizeof *box);
    char *request;
    nurt_status status;
    arguments(env, info, argv);
    pthread_mutex_init(&box->lock, NULL);
    request = string_arg(env, argv[1]);
    status = nurt_exec_start(handle_arg(env, argv[0]), request, collect, box, &box->x);
    free(request);
    if (status != NURT_OK) {
        pthread_mutex_destroy(&box->lock);
        free(box);
        box = NULL;
    }
    return with_handle(env, status, box);
}

static napi_value exec_wait(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS];
    struct exec_box *box;
    nurt_buf out = {0};
    arguments(env, info, argv);
    box = handle_arg(env, argv[0]);
    return result(env, nurt_exec_wait(box ? box->x : NULL, number_arg(env, argv[1]), &out), &out, 1);
}

static napi_value exec_cancel(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS];
    struct exec_box *box;
    arguments(env, info, argv);
    box = handle_arg(env, argv[0]);
    return number(env, (uint32_t)nurt_exec_cancel(box ? box->x : NULL));
}

/* Releases the execution and returns its events as an array of strings. */
static napi_value exec_free(napi_env env, napi_callback_info info) {
    napi_value argv[MAX_ARGS], events, item;
    struct exec_box *box;
    int i;
    arguments(env, info, argv);
    box = handle_arg(env, argv[0]);
    napi_create_array(env, &events);
    if (box == NULL) {
        return events;
    }
    nurt_exec_free(box->x);
    for (i = 0; i < box->count; i++) {
        napi_create_string_utf8(env, box->events[i], NAPI_AUTO_LENGTH, &item);
        napi_set_element(env, events, (uint32_t)i, item);
        free(box->events[i]);
    }
    pthread_mutex_destroy(&box->lock);
    free(box);
    return events;
}

static napi_value init(napi_env env, napi_value exports) {
    static const struct {
        const char *name;
        napi_callback function;
    } functions[] = {
        {"abiVersion", abi_version}, {"abiCheck", abi_check},     {"version", version},
        {"info", lib_info},          {"lastError", last_error},   {"engineOpen", engine_open},
        {"engineClose", engine_close}, {"call", call},            {"execStart", exec_start},
        {"execWait", exec_wait},     {"execCancel", exec_cancel}, {"execFree", exec_free},
    };
    size_t i;
    for (i = 0; i < sizeof functions / sizeof functions[0]; i++) {
        napi_value value;
        napi_create_function(env, functions[i].name, NAPI_AUTO_LENGTH, functions[i].function, NULL, &value);
        napi_set_named_property(env, exports, functions[i].name, value);
    }
    return exports;
}

NAPI_MODULE(NODE_GYP_MODULE_NAME, init)
