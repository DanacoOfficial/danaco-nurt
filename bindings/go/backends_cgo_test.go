// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/go/backends_cgo_test.go
// Responsibility:   Source file bindings/go/backends_cgo_test.go.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//go:build nurt_cgo

package nurt

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

var conformanceBackends = []Backend{BackendProcess, BackendLibrary}

// The stub of libnurt answers a call only when a recorded case has the same
// input. These are the programs of the cases in
// crates/nurt-ffi/tests/fixtures/stub.
const (
	stubCasesDirectory = "../../crates/nurt-ffi/tests/fixtures/stub"
	stubReadProgram    = `find "/srv/app" glob: "*.go"`
	stubEventsProgram  = `find "/srv/app" glob: "*.rs"`
	stubSyntaxProgram  = `find "/srv/app" glob: ["*.go"`
	stubLexicalProgram = `find "/srv/app`
	stubParsedProgram  = `find "/srv/app"`
)

// A language error of parse and check, which the recorded cases do not hold.
const stubLexicalCases = `[
{"description":"Parse of an unterminated text.","function":"parse","input":{"program":"find \"/srv/app","variant":"a"},"output":[null,{"diagnostics":[],"error":{"code":"E_LEXICAL","message":"unterminated text","at":{"line":1,"column":6}}}]},
{"description":"Check of an unterminated text.","function":"check","input":{"nurt":"1.0","program":"find \"/srv/app"},"output":[null,{"diagnostics":[],"error":{"code":"E_LEXICAL","message":"unterminated text","at":{"line":1,"column":6}}}]}
]`

// stubCases copies the recorded cases of the stub into a directory of the
// test, adds the cases of stubLexicalCases and names the directory in
// NURT_STUB_DIR.
func stubCases(t *testing.T) {
	t.Helper()
	dir := t.TempDir()
	files, err := filepath.Glob(filepath.Join(stubCasesDirectory, "*.json"))
	if err != nil || len(files) == 0 {
		t.Fatalf("no recorded cases in %s: %v", stubCasesDirectory, err)
	}
	for _, file := range files {
		data, err := os.ReadFile(file)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, filepath.Base(file)), data, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	var extra []json.RawMessage
	if err := json.Unmarshal([]byte(stubLexicalCases), &extra); err != nil {
		t.Fatal(err)
	}
	for i, c := range extra {
		name := filepath.Join(dir, "lexical-"+string(rune('a'+i))+".json")
		if err := os.WriteFile(name, c, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	t.Setenv("NURT_STUB_DIR", dir)
}

func newLibraryClient(t *testing.T) *Client {
	t.Helper()
	stubCases(t)
	t.Setenv("NURT_STATE_DIR", t.TempDir())
	return newTestClient(t, ClientOptions{Backend: BackendLibrary})
}

func TestLibraryBackendRunsRequest(t *testing.T) {
	c := newLibraryClient(t)
	ctx := context.Background()
	if _, err := c.Info(ctx); err != nil {
		t.Fatalf("Info: %v", err)
	}
	resp, err := c.Run(ctx, Request{Program: stubReadProgram})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if resp.Nurt == "" || resp.Report.Nurt == "" || resp.Report.Kind != "find" || resp.Report.NextCursor != "c_91fe" {
		t.Fatalf("response = %s", resp.Raw)
	}
}

func TestLibraryBackendReturnsLanguageErrorOfRun(t *testing.T) {
	c := newLibraryClient(t)
	resp, err := c.Run(context.Background(), Request{Program: stubSyntaxProgram})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if resp.Report.Error == nil || resp.Report.Error.Code != "E_SYNTAX" {
		t.Fatalf("response = %s", resp.Raw)
	}
}

func TestLibraryBackendRejectsACallWithoutRecordedCase(t *testing.T) {
	c := newLibraryClient(t)
	_, err := c.Run(context.Background(), Request{Program: `find "/srv/unrecorded"`})
	wantStatus(t, err, StatusInternal)
}

func TestLibraryBackendDeliversEvents(t *testing.T) {
	c := newLibraryClient(t)
	ctx := context.Background()
	x, err := c.Start(ctx, Request{Program: stubEventsProgram})
	if err != nil {
		t.Fatal(err)
	}
	defer x.Close()
	var types []string
	for ev := range x.Events() {
		if ev.Time == "" || ev.Seq == 0 {
			t.Errorf("event without time or sequence number: %s", ev.Raw)
		}
		types = append(types, ev.Type)
	}
	if len(types) == 0 || types[0] != "exec.started" || types[len(types)-1] != EventFinished {
		t.Fatalf("event types = %v, want a sequence from exec.started to %s", types, EventFinished)
	}
}

func TestLibraryBackendParseAndCheckReportLanguageErrors(t *testing.T) {
	c := newLibraryClient(t)
	ctx := context.Background()
	parsed, err := c.Parse(ctx, stubLexicalProgram, "")
	if err != nil {
		t.Fatalf("Parse: %v", err)
	}
	if len(parsed.AST) != 0 || parsed.Error == nil || parsed.Error.Code != "E_LEXICAL" {
		t.Fatalf("parse result = %s", parsed.Raw)
	}
	tree, err := c.Parse(ctx, stubParsedProgram, "")
	if err != nil || len(tree.AST) == 0 || tree.Error != nil {
		t.Fatalf("Parse of a valid program = %s, %v", tree.Raw, err)
	}
	checked, err := c.Check(ctx, Request{Program: stubLexicalProgram})
	if err != nil {
		t.Fatalf("Check: %v", err)
	}
	if len(checked.Bound) != 0 || checked.Error == nil {
		t.Fatalf("check result = %s", checked.Raw)
	}
	bound, err := c.Check(ctx, Request{Program: stubReadProgram})
	if err != nil || len(bound.Bound) == 0 || bound.Error != nil {
		t.Fatalf("Check of a valid program = %s, %v", bound.Raw, err)
	}
}

func TestLibraryBackendRejectsUnknownVariant(t *testing.T) {
	c := newLibraryClient(t)
	_, err := c.Parse(context.Background(), stubParsedProgram, "x")
	wantStatus(t, err, StatusArg)
}

func TestLibraryBackendHonoursCancelledContext(t *testing.T) {
	c := newLibraryClient(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, err := c.Run(ctx, Request{Program: stubReadProgram})
	wantStatus(t, err, StatusCancelled)
}

func TestLibraryBackendReleasesExecutions(t *testing.T) {
	c := newLibraryClient(t)
	b, ok := c.b.(*libraryBackend)
	if !ok {
		t.Fatalf("backend is %T", c.b)
	}
	ctx := context.Background()
	const rounds = 50
	for range rounds {
		x, err := c.Start(ctx, Request{Program: stubEventsProgram})
		if err != nil {
			t.Fatal(err)
		}
		for range x.Events() {
		}
		if _, err := x.Wait(ctx); err != nil {
			t.Fatal(err)
		}
		_ = x.Close()
	}
	open, err := c.Start(ctx, Request{Program: stubEventsProgram})
	if err != nil {
		t.Fatal(err)
	}
	if err := c.Close(); err != nil {
		t.Fatal(err)
	}
	b.execMu.Lock()
	live := len(b.execs)
	b.execMu.Unlock()
	if live != 0 {
		t.Fatalf("%d executions not released after Close", live)
	}
	if err := open.Close(); err != nil {
		t.Fatalf("Close of an execution released by the client: %v", err)
	}
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
