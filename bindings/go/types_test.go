// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"

	"github.com/DanacoOfficial/danaco-nurt/bindings/go/internal/rpc"
)

func TestTimeoutUsesLanguageDurationSyntax(t *testing.T) {
	cases := map[time.Duration]string{
		0:                       "",
		-time.Second:            "",
		90 * time.Second:        "90s",
		2 * time.Minute:         "120s",
		1500 * time.Millisecond: "1500ms",
		time.Microsecond:        "1ms",
		1001 * time.Microsecond: "2ms",
	}
	for d, want := range cases {
		if got := formatTimeout(d); got != want {
			t.Errorf("formatTimeout(%v) = %q, want %q", d, got, want)
		}
	}
}

func TestResponseListsTheReportOfEveryPipeline(t *testing.T) {
	var resp Response
	text := `{"nurt":"1.0","report":{"nurt":"1.0","status":"ok","kind":"remove"},` +
		`"reports":[{"nurt":"1.0","status":"ok","kind":"find"},{"nurt":"1.0","status":"ok","kind":"remove"}]}`
	if err := json.Unmarshal([]byte(text), &resp); err != nil {
		t.Fatal(err)
	}
	if len(resp.Reports) != 2 || resp.Reports[0].Kind != "find" || resp.Reports[1].Kind != "remove" {
		t.Fatalf("Reports = %+v, want the reports of find and remove", resp.Reports)
	}
	var single Response
	if err := json.Unmarshal([]byte(`{"nurt":"1.0","report":{"nurt":"1.0","status":"ok","kind":"find"}}`), &single); err != nil {
		t.Fatal(err)
	}
	if len(single.Reports) != 0 {
		t.Fatalf("a single operation has no reports, got %+v", single.Reports)
	}
}

func TestEmptyOptionsAreOmitted(t *testing.T) {
	raw, err := json.Marshal(Options{})
	if err != nil {
		t.Fatal(err)
	}
	if string(raw) != "{}" {
		t.Fatalf("Options{} = %s, want {}", raw)
	}
}

func TestEnvelopeCarriesFormatVersion(t *testing.T) {
	c := &Client{opts: ClientOptions{Session: "s1"}}
	raw, err := json.Marshal(c.envelope(Request{Program: "info", FailOnLanguageError: true}))
	if err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := json.Unmarshal(raw, &got); err != nil {
		t.Fatal(err)
	}
	if got["nurt"] != RequestFormat || got["program"] != "info" {
		t.Fatalf("envelope = %s", raw)
	}
	if _, ok := got["FailOnLanguageError"]; ok {
		t.Fatalf("envelope carries a client-side field: %s", raw)
	}
	if opts, _ := got["options"].(map[string]any); opts["session"] != "s1" {
		t.Fatalf("envelope options = %s", raw)
	}
}

func TestStatusNamesFollowCInterface(t *testing.T) {
	want := map[Status]string{
		StatusOK:        "NURT_OK",
		StatusTimeout:   "NURT_E_TIMEOUT",
		StatusCancelled: "NURT_E_CANCELLED",
		StatusArg:       "NURT_E_ARG",
		StatusABI:       "NURT_E_ABI",
		StatusIO:        "NURT_E_IO",
		StatusInternal:  "NURT_E_INTERNAL",
		Status(7):       "NURT_E_UNKNOWN(7)",
	}
	for s, name := range want {
		if s.String() != name {
			t.Errorf("Status(%d).String() = %q, want %q", int(s), s.String(), name)
		}
	}
}

func TestRPCErrorCodesMapToStatus(t *testing.T) {
	cases := map[int]Status{
		-32001:                 StatusTimeout,
		-32002:                 StatusCancelled,
		-32003:                 StatusArg,
		-32004:                 StatusABI,
		-32005:                 StatusIO,
		-32006:                 StatusInternal,
		appErrorFirst:          StatusInternal,
		rpc.CodeInvalidParams:  StatusArg,
		rpc.CodeMethodNotFound: StatusABI,
		rpc.CodeParseError:     StatusInternal,
		rpc.CodeInvalidRequest: StatusInternal,
		rpc.CodeInternalError:  StatusInternal,
		appErrorLast - 1:       StatusInternal,
		appErrorFirst + 1000:   StatusInternal,
	}
	for code, want := range cases {
		e := statusFromRPC(&rpc.Error{Code: code, Message: "m", Data: json.RawMessage(`{"k":1}`)})
		if e.Status != want {
			t.Errorf("code %d maps to %v, want %v", code, e.Status, want)
		}
		if string(e.Detail) != `{"k":1}` {
			t.Errorf("code %d lost the error data", code)
		}
		var rpcErr *rpc.Error
		if !errors.As(e, &rpcErr) {
			t.Errorf("code %d does not unwrap to the JSON-RPC error", code)
		}
	}
}

func TestRequestIDIsReadFromNumberOrString(t *testing.T) {
	cases := map[string]int64{`7`: 7, `"12"`: 12}
	for raw, want := range cases {
		got, ok := parseRequestID(json.RawMessage(raw))
		if !ok || got != want {
			t.Errorf("parseRequestID(%s) = %d, %v", raw, got, ok)
		}
	}
	for _, raw := range []string{``, `null`, `"x"`, `{}`} {
		if _, ok := parseRequestID(json.RawMessage(raw)); ok {
			t.Errorf("parseRequestID(%s) accepted", raw)
		}
	}
}

func TestEventQueueEndsAfterFinishedEvent(t *testing.T) {
	q := newEventQueue()
	q.push(Event{Type: "exec.started", Seq: 1})
	q.push(Event{Type: EventFinished, Seq: 2})
	q.push(Event{Type: "job.output", Seq: 3})
	var seqs []uint64
	for {
		ev, ok := q.pop()
		if !ok {
			break
		}
		seqs = append(seqs, ev.Seq)
	}
	if len(seqs) != 2 || seqs[0] != 1 || seqs[1] != 2 {
		t.Fatalf("delivered = %v, want 1 and 2 only", seqs)
	}
}

func TestEventQueueDoesNotBlockProducer(t *testing.T) {
	q := newEventQueue()
	const n = 10000
	for i := range n {
		q.push(Event{Type: "job.output", Seq: uint64(i)})
	}
	q.close()
	count := 0
	for {
		if _, ok := q.pop(); !ok {
			break
		}
		count++
	}
	if count != n {
		t.Fatalf("delivered %d events, want %d", count, n)
	}
}

func TestTailBufferKeepsLastBytes(t *testing.T) {
	tb := &tailBuffer{max: 8}
	_, _ = tb.Write([]byte("0123456789"))
	_, _ = tb.Write([]byte("ab"))
	if tb.String() != "456789ab" {
		t.Fatalf("tail = %q", tb.String())
	}
}

func TestErrorMessageNamesStatus(t *testing.T) {
	e := newError(StatusIO, "cannot read", nil)
	if !strings.HasPrefix(e.Error(), "nurt: NURT_E_IO: cannot read") {
		t.Fatalf("Error() = %q", e.Error())
	}
	le := &LanguageError{Code: "E_SYNTAX", Message: "unexpected token"}
	if le.Error() != "nurt: E_SYNTAX: unexpected token" {
		t.Fatalf("Error() = %q", le.Error())
	}
}
