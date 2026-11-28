// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/go/engine_test.go
// Responsibility:   Tests of the process backend against the real engine nurt serve --stdio.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package nurt

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// engineExecutableEnv names the real nurt executable; without it the tests of
// the real engine are skipped.
const engineExecutableEnv = "NURT_TEST_EXECUTABLE"

func engineClient(t *testing.T) *Client {
	t.Helper()
	exe := os.Getenv(engineExecutableEnv)
	if exe == "" {
		t.Skipf("no engine: set %s to the nurt executable", engineExecutableEnv)
	}
	return newTestClient(t, ClientOptions{
		Backend:    BackendProcess,
		Executable: exe,
		Env:        []string{"NURT_STATE_DIR=" + t.TempDir()},
	})
}

func engineJSON(t *testing.T, raw json.RawMessage) map[string]any {
	t.Helper()
	var out map[string]any
	if err := json.Unmarshal(raw, &out); err != nil {
		t.Fatalf("not a JSON object: %s: %v", raw, err)
	}
	return out
}

func engineDirectory(t *testing.T, files int) string {
	t.Helper()
	dir := t.TempDir()
	for i := 0; i < files; i++ {
		name := filepath.Join(dir, "f"+strings.Repeat("x", i)+".txt")
		if err := os.WriteFile(name, []byte("text\n"), 0o600); err != nil {
			t.Fatal(err)
		}
	}
	return dir
}

func engineListing(dir string) string { return `list "` + filepath.ToSlash(dir) + `"` }

func TestEngineInfoReportsTheFormats(t *testing.T) {
	c := engineClient(t)
	raw, err := c.Info(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	info := engineJSON(t, raw)
	formats, _ := info["formats"].(map[string]any)
	if formats["ast"] != "1.1" || formats["bound"] != "1.1" {
		t.Fatalf("formats = %v", formats)
	}
	if _, ok := info["version"].(string); !ok {
		t.Fatalf("info = %s", raw)
	}
}

func TestEngineCatalogSelections(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	whole, err := c.Catalog(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	if got := engineJSON(t, whole)["format"]; got != "nurt-catalog-result/1" {
		t.Fatalf("catalog = %s", whole)
	}
	group, err := c.Catalog(ctx, json.RawMessage(`{"group":"core"}`))
	if err != nil {
		t.Fatal(err)
	}
	if commands, _ := engineJSON(t, group)["commands"].([]any); len(commands) == 0 {
		t.Fatalf("group = %s", group)
	}
	command, err := c.Catalog(ctx, json.RawMessage(`{"command":"list"}`))
	if err != nil {
		t.Fatal(err)
	}
	if spec, _ := engineJSON(t, command)["command"].(map[string]any); spec["name"] != "list" {
		t.Fatalf("command = %s", command)
	}
}

func TestEngineSchemaAndUnknownSchema(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	raw, err := c.Schema(ctx, "ast")
	if err != nil {
		t.Fatal(err)
	}
	if got := engineJSON(t, raw)["title"]; got != "nurt-ast/1" {
		t.Fatalf("schema = %.200s", raw)
	}
	_, err = c.Schema(ctx, "unknown")
	wantStatus(t, err, StatusArg)
}

func TestEngineDescribeReturnsText(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	text, err := c.Describe(ctx, DescribeQuery{})
	if err != nil {
		t.Fatal(err)
	}
	if strings.HasPrefix(text, "{") || !strings.Contains(text, "PROGRAM") {
		t.Fatalf("description = %.200q", text)
	}
	_, err = c.Describe(ctx, DescribeQuery{Name: "no-such-command"})
	var languageError *LanguageError
	if !errors.As(err, &languageError) || languageError.Code != "E_UNKNOWN_NAME" {
		t.Fatalf("an unknown subject must be a language error, got %v", err)
	}
	if _, err := c.Help(ctx, "no-such-topic"); !errors.As(err, &languageError) {
		t.Fatalf("an unknown topic must be a language error, got %v", err)
	}
}

func TestEngineHelpReturnsText(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	index, err := c.Help(ctx, "")
	if err != nil || !strings.HasPrefix(index, "GROUP") {
		t.Fatalf("help = %.200q, %v", index, err)
	}
	topic, err := c.Help(ctx, "list")
	if err != nil || topic == "" {
		t.Fatalf("help list = %.200q, %v", topic, err)
	}
}

func TestEngineRunAndContinuation(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	dir := engineDirectory(t, 5)
	resp, err := c.Run(ctx, Request{Program: engineListing(dir), Options: Options{Limit: 2}})
	if err != nil || !resp.OK() {
		t.Fatalf("Run = %v, %v", resp, err)
	}
	if resp.Report.Kind != "list" || resp.Report.Status != ReportOK {
		t.Fatalf("report = %s", resp.Report.Raw)
	}
	report := engineJSON(t, resp.Report.Raw)
	continuation, _ := report["continuation"].(map[string]any)
	if continuation["id"] == nil || continuation["remaining"] != float64(3) {
		t.Fatalf("continuation = %v", continuation)
	}
	next, err := c.Run(ctx, Request{Program: "next"})
	if err != nil || !next.OK() || next.Report.Kind != "next" {
		t.Fatalf("next = %v, %v", next, err)
	}
	if resp.NextCursor() == "" {
		t.Fatalf("a shortened result must carry a cursor: %s", resp.Raw)
	}
	page, err := c.Page(ctx, resp.NextCursor(), 0)
	if err != nil || !page.OK() {
		t.Fatalf("Page = %v, %v", page, err)
	}
}

func TestEngineLanguageErrorsAreResponses(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	resp, err := c.Run(ctx, Request{Program: `lst "/tmp"`})
	if err != nil || resp.OK() || resp.Report.Error.Code != "E_UNKNOWN_COMMAND" {
		t.Fatalf("Run = %v, %v", resp, err)
	}
	if resp.Report.Error.At == nil || resp.Report.Error.At.Line != 1 {
		t.Fatalf("position = %+v", resp.Report.Error.At)
	}
	page, err := c.Page(ctx, "malformed", 0)
	if err != nil || page.OK() || page.Report.Error.Code != "E_ARG" {
		t.Fatalf("Page = %v, %v", page, err)
	}
}

func TestEngineEventsPrecedeTheResponse(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	x, err := c.Start(ctx, Request{Program: engineListing(engineDirectory(t, 1))})
	if err != nil {
		t.Fatal(err)
	}
	defer x.Close()
	var types []string
	for ev := range x.Events() {
		types = append(types, ev.Type)
	}
	if len(types) < 2 || types[0] != "exec.started" || types[len(types)-1] != EventFinished {
		t.Fatalf("event types = %v", types)
	}
	resp, err := x.Wait(ctx)
	if err != nil || !resp.OK() {
		t.Fatalf("Wait = %v, %v", resp, err)
	}
}

func TestEngineCheckParsePrintAndCorrect(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	check, err := c.Check(ctx, Request{Program: `list "/tmp"`})
	if err != nil || len(check.Bound) == 0 || check.Error != nil {
		t.Fatalf("Check = %+v, %v", check, err)
	}
	parsed, err := c.Parse(ctx, `list "/tmp"`, "")
	if err != nil || parsed.Error != nil || len(parsed.AST) == 0 {
		t.Fatalf("Parse = %+v, %v", parsed, err)
	}
	printed, err := c.Print(ctx, parsed.AST, "")
	if err != nil || !strings.HasPrefix(printed, "list") {
		t.Fatalf("Print = %q, %v", printed, err)
	}
	broken, err := c.Parse(ctx, `list (`, "")
	if err != nil || broken.Error == nil || broken.Error.Code != "E_SYNTAX" {
		t.Fatalf("Parse of a broken program = %+v, %v", broken, err)
	}
	corrected, err := c.Correct(ctx, `list "/tmp"`, "")
	if err != nil {
		t.Fatalf("Correct: %v", err)
	}
	if corrected.Program != `list "/tmp"` {
		t.Fatalf("Correct = %+v", corrected)
	}
	quoted, err := c.Correct(ctx, `find '/tmp'`, "")
	if err != nil || quoted.Program != `find "/tmp"` || len(quoted.Corrections) == 0 {
		t.Fatalf("Correct of a single-quoted path = %+v, %v", quoted, err)
	}
}

func TestEngineHistoryAndSpace(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	for _, query := range []json.RawMessage{nil, json.RawMessage(`{"limit":3}`), json.RawMessage(`"{\"limit\":3}"`)} {
		raw, err := c.History(ctx, query)
		if err != nil {
			t.Fatalf("History(%s): %v", query, err)
		}
		if _, ok := engineJSON(t, raw)["operations"].([]any); !ok {
			t.Fatalf("history = %s", raw)
		}
	}
	space, err := c.Space(ctx, "")
	if err != nil {
		t.Fatal(err)
	}
	if _, ok := engineJSON(t, space)["volumes"].([]any); !ok {
		t.Fatalf("space = %s", space)
	}
}

func TestEnginePrivilegeOptions(t *testing.T) {
	c := engineClient(t)
	ctx := context.Background()
	dir := engineDirectory(t, 1)
	for _, privilege := range []Privilege{PrivilegeAccount, PrivilegeAuto} {
		resp, err := c.Run(ctx, Request{Program: engineListing(dir), Options: Options{Privilege: privilege}})
		if err != nil || !resp.OK() {
			t.Fatalf("Run with privilege %q = %v, %v", privilege, resp, err)
		}
	}
	resp, err := c.Run(ctx, Request{Program: engineListing(dir), Options: Options{AsUser: "root"}})
	if err != nil {
		t.Fatalf("a request with as_user must be a response, not a call failure: %v", err)
	}
	if resp.OK() {
		if engineJSON(t, resp.Report.Raw)["as_user"] != "root" {
			t.Fatalf("report = %s", resp.Report.Raw)
		}
	} else if !strings.HasPrefix(resp.Report.Error.Code, "E_") {
		t.Fatalf("error = %+v", resp.Report.Error)
	}
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
