// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//go:build nurt_cgo

// Package capi calls the C interface of libnurt declared in nurt.h. Every
// function copies the results out of library buffers and releases them
// before returning.
package capi

/*
#cgo pkg-config: nurt
#include <stdint.h>
#include <stdlib.h>
#include <nurt.h>

extern void nurtGoEvent(void *user, char *event_json);

static void nurt_go_event(void *user, const char *event_json) {
	nurtGoEvent(user, (char *)event_json);
}

static nurt_status nurt_go_exec_start(nurt_engine *e, const char *request_json, uintptr_t handle, nurt_exec **out) {
	if (handle == 0) {
		return nurt_exec_start(e, request_json, NULL, NULL, out);
	}
	return nurt_exec_start(e, request_json, nurt_go_event, (void *)handle, out);
}
*/
import "C"

import (
	"bytes"
	"encoding/json"
	"math"
	"runtime"
	"runtime/cgo"
	"strconv"
	"strings"
	"unsafe"
)

// Status codes of nurt_status.
const (
	OK        = int(C.NURT_OK)
	Timeout   = int(C.NURT_E_TIMEOUT)
	Cancelled = int(C.NURT_E_CANCELLED)
	Arg       = int(C.NURT_E_ARG)
	ABI       = int(C.NURT_E_ABI)
	IO        = int(C.NURT_E_IO)
	Internal  = int(C.NURT_E_INTERNAL)
)

// HeaderVersion is the interface version of the nurt.h the package was
// compiled against, in the form returned by nurt_abi_version.
const HeaderVersion = uint32(C.NURT_ABI_MAJOR)<<16 | uint32(C.NURT_ABI_MINOR)

// WaitForever is the timeout of Exec.Wait that waits without limit; it
// equals NURT_WAIT_FOREVER.
const WaitForever = math.MaxUint32

// CallError is a call that returned a status other than NURT_OK.
type CallError struct {
	// Status is the returned nurt_status.
	Status int
	// Record is the JSON returned by nurt_last_error for the call, or nil.
	Record []byte
}

func (e *CallError) Error() string {
	return "libnurt call failed with status " + strconv.Itoa(e.Status)
}

func argError(msg string) *CallError {
	record, _ := json.Marshal(map[string]any{"status": Arg, "code": "NURT_E_ARG", "message": msg})
	return &CallError{Status: Arg, Record: record}
}

// The record of nurt_last_error is kept per thread, so the failing call
// and the query of its record run on one locked OS thread.
func invoke(e *C.nurt_engine, f func() C.nurt_status) error {
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	st := f()
	if st == C.NURT_OK {
		return nil
	}
	// These statuses leave the record of the thread unchanged, so the record
	// found would belong to an earlier failure.
	if st == C.NURT_E_TIMEOUT || st == C.NURT_E_CANCELLED {
		return &CallError{Status: int(st)}
	}
	var buf C.nurt_buf
	defer C.nurt_buf_free(&buf)
	var record []byte
	if C.nurt_last_error(e, &buf) == C.NURT_OK {
		record = take(&buf)
	}
	return &CallError{Status: int(st), Record: record}
}

func take(b *C.nurt_buf) []byte {
	if b.ptr == nil || b.len == 0 {
		return nil
	}
	return bytes.Clone(unsafe.Slice((*byte)(unsafe.Pointer(b.ptr)), int(b.len)))
}

// A Go string may contain a NUL byte, which C would read as the end of the
// text; such text is rejected instead of being truncated.
func cString(s string) (*C.char, error) {
	if strings.IndexByte(s, 0) >= 0 {
		return nil, argError("text contains a NUL byte")
	}
	return C.CString(s), nil
}

// Empty text is passed as NULL, which selects the default of the function.
func optString(s string) (*C.char, error) {
	if s == "" {
		return nil, nil
	}
	return cString(s)
}

func free(p *C.char) {
	if p != nil {
		C.free(unsafe.Pointer(p))
	}
}

// ABIVersion returns nurt_abi_version of the loaded library.
func ABIVersion() uint32 { return uint32(C.nurt_abi_version()) }

// ABICheck calls nurt_abi_check.
func ABICheck(v uint32) error {
	return invoke(nil, func() C.nurt_status { return C.nurt_abi_check(C.uint32_t(v)) })
}

// Info calls nurt_info.
func Info() ([]byte, error) {
	var out C.nurt_buf
	defer C.nurt_buf_free(&out)
	err := invoke(nil, func() C.nurt_status { return C.nurt_info(&out) })
	return take(&out), err
}

// Schema calls nurt_schema.
func Schema(which string) ([]byte, error) {
	w, err := cString(which)
	if err != nil {
		return nil, err
	}
	defer free(w)
	var out C.nurt_buf
	defer C.nurt_buf_free(&out)
	err = invoke(nil, func() C.nurt_status { return C.nurt_schema(w, &out) })
	return take(&out), err
}

// Engine is an engine opened by nurt_engine_open.
type Engine struct {
	ptr *C.nurt_engine
}

// Open calls nurt_engine_open; empty config is passed as NULL.
func Open(config []byte) (*Engine, error) {
	cfg, err := optString(string(config))
	if err != nil {
		return nil, err
	}
	defer free(cfg)
	var e *C.nurt_engine
	if err := invoke(nil, func() C.nurt_status { return C.nurt_engine_open(cfg, &e) }); err != nil {
		return nil, err
	}
	return &Engine{ptr: e}, nil
}

// Close calls nurt_engine_close. The engine must not be used afterwards.
func (e *Engine) Close() {
	C.nurt_engine_close(e.ptr)
	e.ptr = nil
}

// Parse calls nurt_parse.
func (e *Engine) Parse(program string, variant int) (ast, diag []byte, err error) {
	p, err := cString(program)
	if err != nil {
		return nil, nil, err
	}
	defer free(p)
	var a, d C.nurt_buf
	defer C.nurt_buf_free(&a)
	defer C.nurt_buf_free(&d)
	err = invoke(e.ptr, func() C.nurt_status {
		return C.nurt_parse(e.ptr, p, C.nurt_variant(variant), &a, &d)
	})
	return take(&a), take(&d), err
}

// Print calls nurt_print.
func (e *Engine) Print(ast []byte, variant int) ([]byte, error) {
	a, err := cString(string(ast))
	if err != nil {
		return nil, err
	}
	defer free(a)
	var out C.nurt_buf
	defer C.nurt_buf_free(&out)
	err = invoke(e.ptr, func() C.nurt_status { return C.nurt_print(e.ptr, a, C.nurt_variant(variant), &out) })
	return take(&out), err
}

// Correct calls nurt_correct.
func (e *Engine) Correct(program string, variant int) ([]byte, error) {
	p, err := cString(program)
	if err != nil {
		return nil, err
	}
	defer free(p)
	var out C.nurt_buf
	defer C.nurt_buf_free(&out)
	err = invoke(e.ptr, func() C.nurt_status { return C.nurt_correct(e.ptr, p, C.nurt_variant(variant), &out) })
	return take(&out), err
}

// Check calls nurt_check.
func (e *Engine) Check(request []byte) (bound, diag []byte, err error) {
	r, err := cString(string(request))
	if err != nil {
		return nil, nil, err
	}
	defer free(r)
	var b, d C.nurt_buf
	defer C.nurt_buf_free(&b)
	defer C.nurt_buf_free(&d)
	err = invoke(e.ptr, func() C.nurt_status { return C.nurt_check(e.ptr, r, &b, &d) })
	return take(&b), take(&d), err
}

// Run calls nurt_run_json.
func (e *Engine) Run(request []byte) ([]byte, error) {
	r, err := cString(string(request))
	if err != nil {
		return nil, err
	}
	defer free(r)
	var out C.nurt_buf
	defer C.nurt_buf_free(&out)
	err = invoke(e.ptr, func() C.nurt_status { return C.nurt_run_json(e.ptr, r, &out) })
	return take(&out), err
}

// Exec is an execution started by Engine.Start. Wait, Cancel and Free must
// not be called after Free.
type Exec struct {
	ptr    *C.nurt_exec
	handle cgo.Handle
}

// Start calls nurt_exec_start. sink receives a copy of each event and is
// called from a thread of the library; nil requests no events.
func (e *Engine) Start(request []byte, sink func([]byte)) (*Exec, error) {
	r, err := cString(string(request))
	if err != nil {
		return nil, err
	}
	defer free(r)
	var h cgo.Handle
	if sink != nil {
		h = cgo.NewHandle(sink)
	}
	var x *C.nurt_exec
	err = invoke(e.ptr, func() C.nurt_status {
		return C.nurt_go_exec_start(e.ptr, r, C.uintptr_t(h), &x)
	})
	if err != nil {
		if h != 0 {
			h.Delete()
		}
		return nil, err
	}
	return &Exec{ptr: x, handle: h}, nil
}

// Wait calls nurt_exec_wait. With NURT_E_CANCELLED it returns both the
// response of the cancelled execution and the error.
func (x *Exec) Wait(timeoutMS uint32) ([]byte, error) {
	var out C.nurt_buf
	defer C.nurt_buf_free(&out)
	err := invoke(nil, func() C.nurt_status { return C.nurt_exec_wait(x.ptr, C.uint32_t(timeoutMS), &out) })
	return take(&out), err
}

// Cancel calls nurt_exec_cancel.
func (x *Exec) Cancel() error {
	return invoke(nil, func() C.nurt_status { return C.nurt_exec_cancel(x.ptr) })
}

// Free calls nurt_exec_free and releases the event sink. The library makes
// no event call after nurt_exec_free returns.
func (x *Exec) Free() {
	C.nurt_exec_free(x.ptr)
	x.ptr = nil
	if x.handle != 0 {
		x.handle.Delete()
		x.handle = 0
	}
}

// Page calls nurt_page.
func (e *Engine) Page(cursor string, limit uint32) ([]byte, error) {
	c, err := cString(cursor)
	if err != nil {
		return nil, err
	}
	defer free(c)
	var out C.nurt_buf
	defer C.nurt_buf_free(&out)
	err = invoke(e.ptr, func() C.nurt_status { return C.nurt_page(e.ptr, c, C.uint32_t(limit), &out) })
	return take(&out), err
}

// Catalog calls nurt_catalog; empty what is passed as NULL.
func (e *Engine) Catalog(what []byte) ([]byte, error) {
	return e.withOptionalArg(what, func(q *C.char, out *C.nurt_buf) C.nurt_status { return C.nurt_catalog(e.ptr, q, out) })
}

// Describe calls nurt_describe; empty what is passed as NULL.
func (e *Engine) Describe(what []byte) ([]byte, error) {
	return e.withOptionalArg(what, func(q *C.char, out *C.nurt_buf) C.nurt_status { return C.nurt_describe(e.ptr, q, out) })
}

// History calls nurt_history; empty query is passed as NULL.
func (e *Engine) History(query []byte) ([]byte, error) {
	return e.withOptionalArg(query, func(q *C.char, out *C.nurt_buf) C.nurt_status { return C.nurt_history(e.ptr, q, out) })
}

// Help calls nurt_help; an empty topic is passed as NULL.
func (e *Engine) Help(topic string) ([]byte, error) {
	return e.withOptionalArg([]byte(topic), func(q *C.char, out *C.nurt_buf) C.nurt_status { return C.nurt_help(e.ptr, q, out) })
}

// Space calls nurt_space; an empty volume is passed as NULL.
func (e *Engine) Space(volume string) ([]byte, error) {
	return e.withOptionalArg([]byte(volume), func(q *C.char, out *C.nurt_buf) C.nurt_status { return C.nurt_space(e.ptr, q, out) })
}

func (e *Engine) withOptionalArg(arg []byte, f func(*C.char, *C.nurt_buf) C.nurt_status) ([]byte, error) {
	q, err := optString(string(arg))
	if err != nil {
		return nil, err
	}
	defer free(q)
	var out C.nurt_buf
	defer C.nurt_buf_free(&out)
	err = invoke(e.ptr, func() C.nurt_status { return f(q, &out) })
	return take(&out), err
}
