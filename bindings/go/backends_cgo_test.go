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
	"testing"
)

var conformanceBackends = []Backend{BackendProcess, BackendLibrary}

func newLibraryClient(t *testing.T) *Client {
	t.Helper()
	t.Setenv("NURT_STATE_DIR", t.TempDir())
	return newTestClient(t, ClientOptions{Backend: BackendLibrary})
}

func TestLibraryBackendRunsRequest(t *testing.T) {
	c := newLibraryClient(t)
	ctx := context.Background()
	if _, err := c.Info(ctx); err != nil {
		t.Fatalf("Info: %v", err)
	}
	resp, err := c.Run(ctx, Request{Program: "info"})
	if err != nil {
		t.Fatalf("Run: %v", err)
	}
	if resp.Nurt == "" || resp.Report.Nurt == "" {
		t.Fatalf("response = %s", resp.Raw)
	}
}

func TestLibraryBackendDeliversEvents(t *testing.T) {
	c := newLibraryClient(t)
	ctx := context.Background()
	x, err := c.Start(ctx, Request{Program: "info"})
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
	if len(types) == 0 || types[len(types)-1] != EventFinished {
		t.Fatalf("event types = %v, want a sequence that ends with %s", types, EventFinished)
	}
}

func TestLibraryBackendParseAndCheckReportLanguageErrors(t *testing.T) {
	c := newLibraryClient(t)
	ctx := context.Background()
	parsed, err := c.Parse(ctx, `find "bindings`, "")
	if err != nil {
		t.Fatalf("Parse: %v", err)
	}
	if len(parsed.AST) != 0 || parsed.Error == nil || parsed.Error.Code != "E_LEXICAL" {
		t.Fatalf("parse result = %s", parsed.Raw)
	}
	tree, err := c.Parse(ctx, `find "bindings"`, "")
	if err != nil || len(tree.AST) == 0 || tree.Error != nil {
		t.Fatalf("Parse of a valid program = %s, %v", tree.Raw, err)
	}
	checked, err := c.Check(ctx, Request{Program: `find "bindings`})
	if err != nil {
		t.Fatalf("Check: %v", err)
	}
	if len(checked.Bound) != 0 || checked.Error == nil {
		t.Fatalf("check result = %s", checked.Raw)
	}
}

func TestLibraryBackendRejectsUnknownVariant(t *testing.T) {
	c := newLibraryClient(t)
	_, err := c.Parse(context.Background(), "info", "x")
	wantStatus(t, err, StatusArg)
}

func TestLibraryBackendHonoursCancelledContext(t *testing.T) {
	c := newLibraryClient(t)
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, err := c.Run(ctx, Request{Program: "info"})
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
		x, err := c.Start(ctx, Request{Program: "info"})
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
	open, err := c.Start(ctx, Request{Program: "info"})
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
