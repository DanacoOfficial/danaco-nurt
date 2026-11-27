// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"encoding/json"
	"fmt"
	"os"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/DanacoOfficial/danaco-nurt/bindings/go/internal/rpc"
)

// The test binary doubles as a scripted nurt serve: when fakeServerEnv is
// set, TestMain serves the protocol on standard input and output instead of
// running tests. The program text of run requests selects the behaviour:
//
//	crash      exits with status 3 after writing to standard error
//	garbage    writes a line that is not JSON
//	sleep      answers only after cancel or after fakeSleepLimit
//	events     sends exec.started, job.output and exec.finished, then answers
//	error      answers with the language error E_SYNTAX
//	paged      answers with the next cursor "c1"
//	cancelled  answers with the identifiers of cancelled requests
//
// Any other program is echoed: the rendered text is the request envelope.
// fakeModeEnv changes the initialize handshake.
const (
	fakeServerEnv  = "NURT_GO_TEST_FAKE_SERVER"
	fakeModeEnv    = "NURT_GO_TEST_FAKE_MODE"
	fakeSleepLimit = 30 * time.Second

	fakeMaxInflight = 4
	fakeMaxMessage  = 1 << 16

	modeExitOnInitialize = "exit-on-initialize"
	modeOldProtocol      = "old-protocol"
	modeNextABI          = "next-abi"
)

func TestMain(m *testing.M) {
	if os.Getenv(fakeServerEnv) == "1" {
		os.Exit(runFakeServer(os.Args[1:], os.Getenv(fakeModeEnv)))
	}
	os.Exit(m.Run())
}

type fakeRequest struct {
	ID     json.RawMessage `json:"id"`
	Method string          `json:"method"`
	Params json.RawMessage `json:"params"`
}

type fakeServer struct {
	w      *rpc.Writer
	events bool

	mu        sync.Mutex
	waiting   map[string]chan struct{}
	cancelled []string
}

func runFakeServer(args []string, mode string) int {
	framing := rpc.NDJSON
	if slices.Contains(args, "content-length") {
		framing = rpc.ContentLength
	}
	s := &fakeServer{w: rpc.NewWriter(os.Stdout, framing), waiting: make(map[string]chan struct{})}
	r := rpc.NewReader(os.Stdin, framing, fakeMaxMessage)
	var wg sync.WaitGroup
	for {
		msg, err := r.Read()
		if err != nil {
			wg.Wait()
			return 0
		}
		var req fakeRequest
		if err := json.Unmarshal(msg, &req); err != nil {
			return 2
		}
		switch req.Method {
		case "initialize":
			s.initialize(req, mode)
		case "cancel":
			s.cancel(req)
		case "shutdown":
			wg.Wait()
			s.reply(req.ID, struct{}{})
			return 0
		default:
			key := string(req.ID)
			stop := make(chan struct{})
			s.mu.Lock()
			s.waiting[key] = stop
			s.mu.Unlock()
			wg.Add(1)
			go func() {
				defer wg.Done()
				s.handle(req, stop)
				s.mu.Lock()
				delete(s.waiting, key)
				s.mu.Unlock()
			}()
		}
	}
}

func (s *fakeServer) send(v any) {
	msg, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	if err := s.w.Write(msg); err != nil {
		os.Exit(1)
	}
}

func (s *fakeServer) reply(id json.RawMessage, result any) {
	s.send(map[string]any{"jsonrpc": "2.0", "id": id, "result": result})
}

func (s *fakeServer) replyError(id json.RawMessage, code int, msg string) {
	s.send(map[string]any{"jsonrpc": "2.0", "id": id, "error": map[string]any{"code": code, "message": msg}})
}

func (s *fakeServer) initialize(req fakeRequest, mode string) {
	var p initializeParams
	_ = json.Unmarshal(req.Params, &p)
	s.events = p.Capabilities.Events
	protocol, abi := protocolName, uint32(abiRequired)
	switch mode {
	case modeExitOnInitialize:
		fmt.Fprintln(os.Stderr, "fake failure during initialize")
		os.Exit(4)
	case modeOldProtocol:
		protocol = "nurt-rpc/0"
	case modeNextABI:
		abi += 1 << abiMajorShift
	}
	s.reply(req.ID, map[string]any{
		"protocol": protocol,
		"version":  "0.0.0",
		"abi":      abi,
		"features": []string{},
		"limits":   map[string]int{"max_inflight": fakeMaxInflight, "max_message_bytes": fakeMaxMessage},
	})
}

func (s *fakeServer) cancel(req fakeRequest) {
	var p struct {
		RequestID json.RawMessage `json:"request_id"`
	}
	_ = json.Unmarshal(req.Params, &p)
	key := string(p.RequestID)
	s.mu.Lock()
	s.cancelled = append(s.cancelled, key)
	if stop, ok := s.waiting[key]; ok {
		close(stop)
		delete(s.waiting, key)
	}
	s.mu.Unlock()
	s.reply(req.ID, struct{}{})
}

func report(status, kind string) map[string]any {
	return map[string]any{"nurt": "1.0", "status": status, "kind": kind}
}

func response(rep map[string]any, text string) map[string]any {
	return map[string]any{"nurt": "1.0", "report": rep, "rendered": map[string]any{"format": "text", "text": text}}
}

func (s *fakeServer) handle(req fakeRequest, stop <-chan struct{}) {
	var p map[string]any
	_ = json.Unmarshal(req.Params, &p)
	str := func(k string) string {
		v, _ := p[k].(string)
		return v
	}
	switch req.Method {
	case "run":
		s.run(req, str("program"), stop)
	case "check":
		s.reply(req.ID, map[string]any{"bound": map[string]any{"nurt": "1.0"}, "diagnostics": []any{}})
	case "parse":
		s.reply(req.ID, map[string]any{"ast": map[string]any{"nurt": "1.0", "program": []any{}}, "diagnostics": []any{}})
	case "print":
		s.reply(req.ID, map[string]any{"text": "print " + str("variant")})
	case "correct":
		s.reply(req.ID, map[string]any{"program": str("program") + " corrected", "corrections": []any{}})
	case "page":
		s.reply(req.ID, response(report("ok", "page"), "page "+str("cursor")))
	case "describe":
		s.reply(req.ID, map[string]any{"description": "describe " + str("subject") + " " + str("language")})
	case "help":
		s.reply(req.ID, map[string]any{"text": "help " + str("topic")})
	case "schema":
		if str("name") == "unknown" {
			s.replyError(req.ID, appErrorFirst-int(StatusArg), "unknown schema")
			return
		}
		s.reply(req.ID, map[string]any{"$id": str("name")})
	case "catalog", "history", "space":
		s.reply(req.ID, map[string]any{"method": req.Method, "params": p})
	case "info":
		s.reply(req.ID, map[string]any{"version": "0.0.0", "features": []string{}})
	default:
		s.replyError(req.ID, rpc.CodeMethodNotFound, "method not found: "+req.Method)
	}
}

func (s *fakeServer) run(req fakeRequest, program string, stop <-chan struct{}) {
	switch program {
	case "crash":
		fmt.Fprintln(os.Stderr, "fatal: simulated failure")
		os.Exit(3)
	case "garbage":
		_ = s.w.Write([]byte("this is not json"))
	case "sleep":
		select {
		case <-stop:
		case <-time.After(fakeSleepLimit):
		}
		s.reply(req.ID, response(report("rolled_back", "run"), "stopped"))
	case "events":
		if s.events {
			for i, t := range []string{"exec.started", "job.output", EventFinished} {
				s.send(map[string]any{"jsonrpc": "2.0", "method": eventMethod, "params": map[string]any{
					"t": t, "seq": i + 1, "time": "2026-01-01T00:00:00Z", "request": string(req.ID),
				}})
			}
		}
		s.reply(req.ID, response(report("ok", "run"), "done"))
	case "error":
		rep := report("failed", "run")
		rep["error"] = map[string]any{"code": "E_SYNTAX", "message": "unexpected token", "at": map[string]int{"line": 1, "column": 5}}
		s.reply(req.ID, response(rep, "error"))
	case "paged":
		rep := report("ok", "find")
		rep["next_cursor"] = "c1"
		s.reply(req.ID, response(rep, "first page"))
	case "cancelled":
		s.mu.Lock()
		text := strings.Join(s.cancelled, ",")
		s.mu.Unlock()
		s.reply(req.ID, response(report("ok", "run"), text))
	default:
		s.reply(req.ID, response(report("ok", "run"), string(req.Params)))
	}
}
