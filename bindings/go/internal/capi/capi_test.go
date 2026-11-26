// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//go:build nurt_cgo

package capi

import (
	"encoding/json"
	"errors"
	"slices"
	"testing"
)

// The table of possible status codes of each function, as declared in the
// comment of nurt.h. A call outside its row is a violation of the
// interface.
var allowed = map[string][]int{
	"nurt_abi_check":   {OK, ABI},
	"nurt_info":        {OK, Arg, Internal},
	"nurt_engine_open": {OK, Arg, IO, Internal},
	"nurt_parse":       {OK, Arg, Internal},
	"nurt_print":       {OK, Arg, Internal},
	"nurt_correct":     {OK, Arg, Internal},
	"nurt_check":       {OK, Arg, Internal},
	"nurt_run_json":    {OK, Arg, Internal},
	"nurt_exec_start":  {OK, Arg, Internal},
	"nurt_exec_wait":   {OK, Timeout, Cancelled, Arg, Internal},
	"nurt_exec_cancel": {OK, Arg, Internal},
	"nurt_page":        {OK, Arg, Internal},
	"nurt_catalog":     {OK, Arg, Internal},
	"nurt_describe":    {OK, Arg, Internal},
	"nurt_help":        {OK, Arg, Internal},
	"nurt_schema":      {OK, Arg, Internal},
	"nurt_history":     {OK, Arg, Internal},
	"nurt_space":       {OK, Arg, Internal},
}

const (
	variantA     = 1
	invalidUTF8  = "\xff\xfe"
	notAnObject  = "[]"
	validRequest = `{"nurt":"1.0","program":"info"}`
)

func status(err error) int {
	if err == nil {
		return OK
	}
	var ce *CallError
	if errors.As(err, &ce) {
		return ce.Status
	}
	return -1
}

func expectIn(t *testing.T, fn string, err error) int {
	t.Helper()
	s := status(err)
	if !slices.Contains(allowed[fn], s) {
		t.Errorf("%s returned status %d, outside %v", fn, s, allowed[fn])
	}
	return s
}

func expectStatus(t *testing.T, fn string, err error, want int) {
	t.Helper()
	if s := expectIn(t, fn, err); s != want {
		t.Errorf("%s returned status %d, want %d", fn, s, want)
	}
}

func openEngine(t *testing.T) *Engine {
	t.Helper()
	e, err := Open(nil)
	if err != nil {
		t.Fatalf("nurt_engine_open: %v", err)
	}
	t.Cleanup(e.Close)
	return e
}

func TestABIVersionMatchesHeader(t *testing.T) {
	v := ABIVersion()
	if v>>16 != HeaderVersion>>16 || v&0xffff < HeaderVersion&0xffff {
		t.Fatalf("library interface %#x is not compatible with header %#x", v, HeaderVersion)
	}
	expectStatus(t, "nurt_abi_check", ABICheck(HeaderVersion), OK)
	expectStatus(t, "nurt_abi_check", ABICheck(HeaderVersion+1<<16), ABI)
}

func TestStatusMatrixForInvalidArguments(t *testing.T) {
	e := openEngine(t)

	_, err := Open([]byte(notAnObject))
	expectStatus(t, "nurt_engine_open", err, Arg)

	_, _, err = e.Parse(invalidUTF8, variantA)
	expectStatus(t, "nurt_parse", err, Arg)

	_, err = e.Print([]byte("not json"), variantA)
	expectStatus(t, "nurt_print", err, Arg)

	_, err = e.Correct(invalidUTF8, variantA)
	expectStatus(t, "nurt_correct", err, Arg)

	_, _, err = e.Check([]byte(notAnObject))
	expectStatus(t, "nurt_check", err, Arg)

	_, err = e.Run([]byte(notAnObject))
	expectStatus(t, "nurt_run_json", err, Arg)

	_, err = e.Start([]byte(notAnObject), nil)
	expectStatus(t, "nurt_exec_start", err, Arg)

	_, err = e.Page(invalidUTF8, 0)
	expectStatus(t, "nurt_page", err, Arg)

	_, err = e.Catalog([]byte(notAnObject))
	expectStatus(t, "nurt_catalog", err, Arg)

	_, err = e.Describe([]byte(notAnObject))
	expectStatus(t, "nurt_describe", err, Arg)

	_, err = e.Help(invalidUTF8)
	expectStatus(t, "nurt_help", err, Arg)

	_, err = Schema("unknown")
	expectStatus(t, "nurt_schema", err, Arg)

	_, err = e.History([]byte(notAnObject))
	expectStatus(t, "nurt_history", err, Arg)

	_, err = e.Space(invalidUTF8)
	expectStatus(t, "nurt_space", err, Arg)
}

func TestStatusMatrixForValidCalls(t *testing.T) {
	e := openEngine(t)

	_, err := Info()
	expectStatus(t, "nurt_info", err, OK)

	_, err = Schema("request")
	expectStatus(t, "nurt_schema", err, OK)

	_, _, err = e.Parse("info", variantA)
	expectStatus(t, "nurt_parse", err, OK)

	_, err = e.Correct("info", variantA)
	expectStatus(t, "nurt_correct", err, OK)

	_, _, err = e.Check([]byte(validRequest))
	expectStatus(t, "nurt_check", err, OK)

	_, err = e.Run([]byte(validRequest))
	expectStatus(t, "nurt_run_json", err, OK)

	_, err = e.Catalog(nil)
	expectStatus(t, "nurt_catalog", err, OK)

	_, err = e.Describe(nil)
	expectStatus(t, "nurt_describe", err, OK)

	_, err = e.Help("")
	expectStatus(t, "nurt_help", err, OK)

	_, err = e.History(nil)
	expectStatus(t, "nurt_history", err, OK)

	_, err = e.Space("")
	expectStatus(t, "nurt_space", err, OK)
}

func TestExecutionStatuses(t *testing.T) {
	e := openEngine(t)
	x, err := e.Start([]byte(validRequest), nil)
	expectStatus(t, "nurt_exec_start", err, OK)
	if err != nil {
		return
	}
	defer x.Free()

	_, err = x.Wait(0)
	expectIn(t, "nurt_exec_wait", err)
	expectStatus(t, "nurt_exec_cancel", x.Cancel(), OK)
	expectStatus(t, "nurt_exec_cancel", x.Cancel(), OK)

	resp, err := x.Wait(WaitForever)
	s := expectIn(t, "nurt_exec_wait", err)
	if s != OK && s != Cancelled {
		t.Fatalf("nurt_exec_wait after cancellation returned status %d", s)
	}
	if len(resp) == 0 {
		t.Fatal("nurt_exec_wait returned no response after completion")
	}
	again, err := x.Wait(0)
	if status(err) != s || string(again) != string(resp) {
		t.Fatal("nurt_exec_wait returned a different result when called again")
	}
}

func TestEventsAreDeliveredUntilFinished(t *testing.T) {
	e := openEngine(t)
	events := make(chan []byte, 1024)
	x, err := e.Start([]byte(validRequest), func(ev []byte) { events <- ev })
	if err != nil {
		t.Fatalf("nurt_exec_start: %v", err)
	}
	if _, err := x.Wait(WaitForever); err != nil {
		t.Fatalf("nurt_exec_wait: %v", err)
	}
	x.Free()
	close(events)
	var last struct {
		T string `json:"t"`
	}
	n := 0
	for ev := range events {
		n++
		if err := json.Unmarshal(ev, &last); err != nil {
			t.Fatalf("event is not JSON: %s", ev)
		}
	}
	if n > 0 && last.T != "exec.finished" {
		t.Fatalf("last event is %q, want exec.finished", last.T)
	}
}

func TestLastErrorDescribesFailure(t *testing.T) {
	_, err := Schema("unknown")
	var ce *CallError
	if !errors.As(err, &ce) {
		t.Fatalf("error = %v, want *CallError", err)
	}
	var rec struct {
		Status  int    `json:"status"`
		Code    string `json:"code"`
		Message string `json:"message"`
	}
	if err := json.Unmarshal(ce.Record, &rec); err != nil {
		t.Fatalf("nurt_last_error record is not JSON: %q", ce.Record)
	}
	if rec.Status != Arg || rec.Code != "NURT_E_ARG" || rec.Message == "" {
		t.Fatalf("nurt_last_error record = %+v", rec)
	}
}

func TestCancelledWaitCarriesNoRecordOfAnEarlierFailure(t *testing.T) {
	e := openEngine(t)
	// A failing call leaves a record on the thread; the status of a
	// cancelled wait must not pick it up.
	_, err := Schema("unknown")
	expectStatus(t, "nurt_schema", err, Arg)
	x, err := e.Start([]byte(validRequest), nil)
	expectStatus(t, "nurt_exec_start", err, OK)
	if err != nil {
		return
	}
	defer x.Free()
	expectStatus(t, "nurt_exec_cancel", x.Cancel(), OK)
	_, err = x.Wait(WaitForever)
	var ce *CallError
	if errors.As(err, &ce) && ce.Record != nil {
		t.Fatalf("status %d carries the record %s", ce.Status, ce.Record)
	}
}

func TestNULByteIsRejectedBeforeTheCall(t *testing.T) {
	e := openEngine(t)
	_, _, err := e.Parse("info\x00more", variantA)
	expectStatus(t, "nurt_parse", err, Arg)
}
