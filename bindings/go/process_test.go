// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/go/process_test.go
// Responsibility:   Source file bindings/go/process_test.go.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package nurt

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"
)

const shortDeadline = 100 * time.Millisecond

func fakeOptions(t *testing.T, mode string) ClientOptions {
	t.Helper()
	exe, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	return ClientOptions{
		Executable: exe,
		Env:        []string{fakeServerEnv + "=1", fakeModeEnv + "=" + mode},
	}
}

func newTestClient(t *testing.T, opts ClientOptions) *Client {
	t.Helper()
	c, err := New(context.Background(), opts)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	t.Cleanup(func() { _ = c.Close() })
	return c
}

func wantStatus(t *testing.T, err error, want Status) *Error {
	t.Helper()
	var e *Error
	if !errors.As(err, &e) {
		t.Fatalf("error = %v, want *Error with %v", err, want)
	}
	if e.Status != want {
		t.Fatalf("status = %v (%s), want %v", e.Status, e.Message, want)
	}
	return e
}

func echoedEnvelope(t *testing.T, resp *Response) map[string]any {
	t.Helper()
	var env map[string]any
	if err := json.Unmarshal([]byte(resp.Text()), &env); err != nil {
		t.Fatalf("echoed text is not JSON: %q", resp.Text())
	}
	return env
}

func TestRunReturnsResponse(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	resp, err := c.Run(context.Background(), Request{Program: "info"})
	if err != nil {
		t.Fatal(err)
	}
	if !resp.OK() || resp.Err() != nil {
		t.Fatalf("response reports an error: %s", resp.Raw)
	}
	if resp.Report.Status != ReportOK || resp.Report.Kind != "run" || len(resp.Report.Raw) == 0 {
		t.Fatalf("report = %+v", resp.Report)
	}
	env := echoedEnvelope(t, resp)
	if env["nurt"] != RequestFormat || env["program"] != "info" {
		t.Fatalf("envelope = %v", env)
	}
}

func TestRunAppliesClientDefaultsAndOptions(t *testing.T) {
	opts := fakeOptions(t, "")
	opts.Session = "s-client"
	opts.Language = "pl"
	c := newTestClient(t, opts)
	zero := 0
	resp, err := c.Run(context.Background(), Request{
		Program: "info",
		Options: Options{Limit: 10, DiffContext: &zero, Timeout: 1500 * time.Millisecond},
	})
	if err != nil {
		t.Fatal(err)
	}
	got, _ := echoedEnvelope(t, resp)["options"].(map[string]any)
	want := map[string]any{"limit": 10.0, "diff_context": 0.0, "timeout": "1500ms", "session": "s-client", "language": "pl"}
	for k, v := range want {
		if got[k] != v {
			t.Errorf("options[%q] = %v, want %v", k, got[k], v)
		}
	}

	resp, err = c.Run(context.Background(), Request{Program: "info", Options: Options{Session: "s-request"}})
	if err != nil {
		t.Fatal(err)
	}
	got, _ = echoedEnvelope(t, resp)["options"].(map[string]any)
	if got["session"] != "s-request" {
		t.Fatalf("session = %v, want the session of the request", got["session"])
	}
}

func TestLanguageErrorIsPartOfResponse(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	resp, err := c.Run(context.Background(), Request{Program: "error"})
	if err != nil {
		t.Fatalf("language error returned as call failure: %v", err)
	}
	if resp.OK() {
		t.Fatal("response with a language error reports OK")
	}
	var le *LanguageError
	if !errors.As(resp.Err(), &le) {
		t.Fatalf("Err() = %v, want *LanguageError", resp.Err())
	}
	if le.Code != "E_SYNTAX" || le.At == nil || le.At.Line != 1 || le.At.Column != 5 || len(le.Raw) == 0 {
		t.Fatalf("language error = %+v", le)
	}
}

func TestFailOnLanguageErrorReturnsErrorWithResponse(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	resp, err := c.Run(context.Background(), Request{Program: "error", FailOnLanguageError: true})
	var le *LanguageError
	if !errors.As(err, &le) || le.Code != "E_SYNTAX" {
		t.Fatalf("error = %v, want *LanguageError E_SYNTAX", err)
	}
	if resp == nil || resp.OK() {
		t.Fatal("response missing together with the language error")
	}
}

func TestPageFollowsCursor(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx := context.Background()
	resp, err := c.Run(ctx, Request{Program: "paged"})
	if err != nil {
		t.Fatal(err)
	}
	if resp.NextCursor() != "c1" {
		t.Fatalf("next cursor = %q", resp.NextCursor())
	}
	next, err := c.Page(ctx, resp.NextCursor(), 0)
	if err != nil {
		t.Fatal(err)
	}
	if next.Text() != "page c1" {
		t.Fatalf("page text = %q", next.Text())
	}
	_, err = c.Page(ctx, "c1", -1)
	wantStatus(t, err, StatusArg)
}

func TestEveryMethodReachesServer(t *testing.T) {
	opts := fakeOptions(t, "")
	opts.Language = "en"
	c := newTestClient(t, opts)
	ctx := context.Background()

	if res, err := c.Check(ctx, Request{Program: "info"}); err != nil || len(res.Bound) == 0 {
		t.Errorf("Check = %+v, %v", res, err)
	}
	if res, err := c.Parse(ctx, "info", ""); err != nil || len(res.AST) == 0 {
		t.Errorf("Parse = %+v, %v", res, err)
	}
	if text, err := c.Print(ctx, json.RawMessage(`{"nurt":"1.0","program":[]}`), VariantB); err != nil || text != "print b" {
		t.Errorf("Print = %q, %v", text, err)
	}
	if res, err := c.Correct(ctx, "info", VariantA); err != nil || res.Program != "info corrected" || len(res.Raw) == 0 {
		t.Errorf("Correct = %+v, %v", res, err)
	}
	if text, err := c.Describe(ctx, DescribeQuery{Layer: "core"}); err != nil || text != "describe core en" {
		t.Errorf("Describe = %q, %v", text, err)
	}
	if text, err := c.Help(ctx, "paths"); err != nil || text != "help paths" {
		t.Errorf("Help = %q, %v", text, err)
	}
	if raw, err := c.Schema(ctx, "report"); err != nil || !strings.Contains(string(raw), `"report"`) {
		t.Errorf("Schema = %s, %v", raw, err)
	}
	if raw, err := c.Info(ctx); err != nil || !strings.Contains(string(raw), "version") {
		t.Errorf("Info = %s, %v", raw, err)
	}
	for name, call := range map[string]func() (json.RawMessage, error){
		"catalog": func() (json.RawMessage, error) { return c.Catalog(ctx, nil) },
		"history": func() (json.RawMessage, error) { return c.History(ctx, json.RawMessage(`{"limit":5}`)) },
		"space":   func() (json.RawMessage, error) { return c.Space(ctx, "/home/user") },
	} {
		raw, err := call()
		if err != nil || !strings.Contains(string(raw), `"method":"`+name+`"`) {
			t.Errorf("%s = %s, %v", name, raw, err)
		}
	}
}

func TestApplicationErrorCodeMapsToStatus(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	_, err := c.Schema(context.Background(), "unknown")
	e := wantStatus(t, err, StatusArg)
	if e.Message != "unknown schema" {
		t.Fatalf("message = %q", e.Message)
	}
}

func TestConcurrentRunsReceiveTheirOwnResponses(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	const n = 32
	var wg sync.WaitGroup
	errs := make(chan error, n)
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			program := fmt.Sprintf("echo %d", i)
			resp, err := c.Run(context.Background(), Request{Program: program})
			if err != nil {
				errs <- err
				return
			}
			if !strings.Contains(resp.Text(), `"program":"`+program+`"`) {
				errs <- fmt.Errorf("request %q received %s", program, resp.Text())
			}
		}()
	}
	wg.Wait()
	close(errs)
	for err := range errs {
		t.Error(err)
	}
}

func TestEventsAreDeliveredInOrder(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx := context.Background()
	x, err := c.Start(ctx, Request{Program: "events"})
	if err != nil {
		t.Fatal(err)
	}
	defer x.Close()
	var types []string
	var seqs []uint64
	for ev := range x.Events() {
		types = append(types, ev.Type)
		seqs = append(seqs, ev.Seq)
	}
	if !slices.Equal(types, []string{"exec.started", "job.output", EventFinished}) {
		t.Fatalf("event types = %v", types)
	}
	if !slices.IsSorted(seqs) {
		t.Fatalf("event sequence = %v", seqs)
	}
	resp, err := x.Wait(ctx)
	if err != nil || !resp.OK() {
		t.Fatalf("Wait = %v, %v", resp, err)
	}
}

func TestCancelReturnsResponseOfCancelledExecution(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx := context.Background()
	x, err := c.Start(ctx, Request{Program: "sleep"})
	if err != nil {
		t.Fatal(err)
	}
	defer x.Close()
	if err := x.Cancel(ctx); err != nil {
		t.Fatal(err)
	}
	if err := x.Cancel(ctx); err != nil {
		t.Fatalf("second Cancel: %v", err)
	}
	resp, err := x.Wait(ctx)
	wantStatus(t, err, StatusCancelled)
	if resp == nil || resp.Report.Status != ReportRolledBack {
		t.Fatalf("response = %+v", resp)
	}
	for range x.Events() {
		t.Fatal("cancelled execution without events produced an event")
	}
}

func TestWaitWithEndedContextKeepsExecution(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	x, err := c.Start(context.Background(), Request{Program: "sleep"})
	if err != nil {
		t.Fatal(err)
	}
	defer x.Close()
	ctx, cancel := context.WithTimeout(context.Background(), shortDeadline)
	defer cancel()
	_, err = x.Wait(ctx)
	if e := wantStatus(t, err, StatusTimeout); !errors.Is(e, context.DeadlineExceeded) {
		t.Fatalf("error %v does not unwrap to context.DeadlineExceeded", e)
	}
	if err := x.Cancel(context.Background()); err != nil {
		t.Fatal(err)
	}
	if _, err := x.Wait(context.Background()); err == nil {
		t.Fatal("Wait after Cancel returned no error")
	}
}

func TestContextDeadlineCancelsRequest(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx, cancel := context.WithTimeout(context.Background(), shortDeadline)
	defer cancel()
	_, err := c.Run(ctx, Request{Program: "sleep"})
	if e := wantStatus(t, err, StatusTimeout); !errors.Is(e, context.DeadlineExceeded) {
		t.Fatalf("error %v does not unwrap to context.DeadlineExceeded", e)
	}
	resp, err := c.Run(context.Background(), Request{Program: "cancelled"})
	if err != nil {
		t.Fatal(err)
	}
	if resp.Text() == "" {
		t.Fatal("the request was not cancelled in nurt")
	}
}

func TestCancelledContextFailsBeforeSending(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	_, err := c.Run(ctx, Request{Program: "info"})
	if e := wantStatus(t, err, StatusCancelled); !errors.Is(e, context.Canceled) {
		t.Fatalf("error %v does not unwrap to context.Canceled", e)
	}
	_, err = c.Start(ctx, Request{Program: "info"})
	wantStatus(t, err, StatusCancelled)
}

func TestExecutionCloseCancelsRunningRequest(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx := context.Background()
	x, err := c.Start(ctx, Request{Program: "sleep"})
	if err != nil {
		t.Fatal(err)
	}
	if err := x.Close(); err != nil {
		t.Fatal(err)
	}
	if err := x.Close(); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	_, err = x.Wait(ctx)
	wantStatus(t, err, StatusArg)
	resp, err := c.Run(ctx, Request{Program: "cancelled"})
	if err != nil {
		t.Fatal(err)
	}
	if resp.Text() == "" {
		t.Fatal("closing the execution did not cancel the request")
	}
}

func TestProcessFailureFailsCallAndProcessIsRestarted(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx := context.Background()
	_, err := c.Run(ctx, Request{Program: "crash"})
	e := wantStatus(t, err, StatusInternal)
	if !strings.Contains(e.Message, "exit status 3") || !strings.Contains(e.Message, "simulated failure") {
		t.Fatalf("message = %q, want the exit status and standard error", e.Message)
	}
	if _, err := c.Run(ctx, Request{Program: "info"}); err != nil {
		t.Fatalf("call after restart: %v", err)
	}
}

func TestProcessFailureFailsExecutionsInFlight(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx := context.Background()
	x, err := c.Start(ctx, Request{Program: "sleep"})
	if err != nil {
		t.Fatal(err)
	}
	defer x.Close()
	_, err = c.Run(ctx, Request{Program: "crash"})
	wantStatus(t, err, StatusInternal)
	_, err = x.Wait(ctx)
	wantStatus(t, err, StatusInternal)
}

func TestInvalidOutputEndsProcess(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx := context.Background()
	_, err := c.Run(ctx, Request{Program: "garbage"})
	e := wantStatus(t, err, StatusInternal)
	if !strings.Contains(e.Message, "invalid message") {
		t.Fatalf("message = %q", e.Message)
	}
	if _, err := c.Run(ctx, Request{Program: "info"}); err != nil {
		t.Fatalf("call after restart: %v", err)
	}
}

func TestIncompatibleServerIsRejected(t *testing.T) {
	for _, mode := range []string{modeOldProtocol, modeNextABI} {
		t.Run(mode, func(t *testing.T) {
			_, err := New(context.Background(), fakeOptions(t, mode))
			wantStatus(t, err, StatusABI)
		})
	}
}

func TestStartupFailureReportsExitStatusAndStandardError(t *testing.T) {
	_, err := New(context.Background(), fakeOptions(t, modeExitOnInitialize))
	e := wantStatus(t, err, StatusInternal)
	if !strings.Contains(e.Message, "exit status 4") || !strings.Contains(e.Message, "fake failure during initialize") {
		t.Fatalf("message = %q", e.Message)
	}
}

func TestMissingExecutableIsReported(t *testing.T) {
	missing := filepath.Join(t.TempDir(), "nurt")
	_, err := New(context.Background(), ClientOptions{Executable: missing})
	e := wantStatus(t, err, StatusIO)
	if len(e.Checked) == 0 || !strings.Contains(e.Error(), missing) {
		t.Fatalf("error = %v, want the checked location", e)
	}
}

func TestContentLengthFraming(t *testing.T) {
	opts := fakeOptions(t, "")
	opts.Framing = FramingContentLength
	c := newTestClient(t, opts)
	resp, err := c.Run(context.Background(), Request{Program: "multi\nline"})
	if err != nil {
		t.Fatal(err)
	}
	if echoedEnvelope(t, resp)["program"] != "multi\nline" {
		t.Fatalf("text = %q", resp.Text())
	}
}

func TestRequestAboveServerLimitIsRejected(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	_, err := c.Run(context.Background(), Request{Program: strings.Repeat("x", fakeMaxMessage)})
	wantStatus(t, err, StatusArg)
	if _, err := c.Run(context.Background(), Request{Program: "info"}); err != nil {
		t.Fatalf("call after a rejected request: %v", err)
	}
}

func TestClosedClientRejectsCalls(t *testing.T) {
	c, err := New(context.Background(), fakeOptions(t, ""))
	if err != nil {
		t.Fatal(err)
	}
	if err := c.Close(); err != nil {
		t.Fatal(err)
	}
	if err := c.Close(); err != nil {
		t.Fatalf("second Close: %v", err)
	}
	_, err = c.Run(context.Background(), Request{Program: "info"})
	wantStatus(t, err, StatusArg)
}

func TestUnknownBackendIsRejected(t *testing.T) {
	_, err := New(context.Background(), ClientOptions{Backend: "remote"})
	wantStatus(t, err, StatusArg)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
