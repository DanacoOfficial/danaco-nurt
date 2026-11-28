// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/go/internal/rpc/conn_test.go
// Responsibility:   Source file bindings/go/internal/rpc/conn_test.go.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package rpc

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"sync"
	"testing"
	"time"
)

const peerMax = 1 << 20

// The peer is the server end of a connection under test.
type peer struct {
	r       *Reader
	w       *Writer
	fromSrv *io.PipeWriter
}

type peerMessage struct {
	ID     int64           `json:"id"`
	Method string          `json:"method"`
	Params json.RawMessage `json:"params"`
	Error  *Error          `json:"error"`
}

func newPair(t *testing.T, notify NotifyFunc) (*Conn, *peer) {
	t.Helper()
	clientIn, serverOut := io.Pipe()
	serverIn, clientOut := io.Pipe()
	c := NewConn(clientIn, clientOut, NDJSON, peerMax, notify)
	p := &peer{r: NewReader(serverIn, NDJSON, peerMax), w: NewWriter(serverOut, NDJSON), fromSrv: serverOut}
	t.Cleanup(func() {
		_ = serverOut.Close()
		_ = clientOut.Close()
	})
	return c, p
}

func (p *peer) next(t *testing.T) peerMessage {
	t.Helper()
	raw, err := p.r.Read()
	if err != nil {
		t.Errorf("peer read: %v", err)
		return peerMessage{}
	}
	var m peerMessage
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Errorf("peer decode: %v", err)
	}
	return m
}

func (p *peer) send(t *testing.T, v any) {
	t.Helper()
	raw, err := json.Marshal(v)
	if err != nil {
		t.Errorf("peer encode: %v", err)
		return
	}
	if err := p.w.Write(raw); err != nil {
		t.Errorf("peer write: %v", err)
	}
}

func result(id int64, v any) map[string]any {
	return map[string]any{"jsonrpc": "2.0", "id": id, "result": v}
}

func TestCallMatchesResponsesOutOfOrder(t *testing.T) {
	c, p := newPair(t, nil)
	const n = 8
	var wg sync.WaitGroup
	got := make([]string, n)
	errs := make([]error, n)
	for i := range n {
		wg.Add(1)
		go func() {
			defer wg.Done()
			res, err := c.Call(context.Background(), "echo", map[string]int{"i": i})
			errs[i] = err
			got[i] = string(res)
		}()
	}
	msgs := make([]peerMessage, n)
	for i := range n {
		msgs[i] = p.next(t)
	}
	for i := n - 1; i >= 0; i-- {
		p.send(t, result(msgs[i].ID, json.RawMessage(msgs[i].Params)))
	}
	wg.Wait()
	for i := range n {
		if errs[i] != nil {
			t.Fatalf("call %d: %v", i, errs[i])
		}
		var params map[string]int
		if err := json.Unmarshal([]byte(got[i]), &params); err != nil || params["i"] != i {
			t.Fatalf("call %d received %s", i, got[i])
		}
	}
}

func TestErrorResponseIsReturnedAsError(t *testing.T) {
	c, p := newPair(t, nil)
	go func() {
		m := p.next(t)
		p.send(t, map[string]any{"jsonrpc": "2.0", "id": m.ID, "error": map[string]any{"code": -32602, "message": "bad params"}})
	}()
	_, err := c.Call(context.Background(), "run", nil)
	var rpcErr *Error
	if !errors.As(err, &rpcErr) || rpcErr.Code != CodeInvalidParams {
		t.Fatalf("error = %v, want code %d", err, CodeInvalidParams)
	}
}

func TestNotificationsAreDeliveredInOrder(t *testing.T) {
	var mu sync.Mutex
	var seen []string
	c, p := newPair(t, func(method string, params json.RawMessage) {
		mu.Lock()
		seen = append(seen, method+string(params))
		mu.Unlock()
	})
	go func() {
		m := p.next(t)
		p.send(t, map[string]any{"jsonrpc": "2.0", "method": "nurt/event", "params": map[string]int{"seq": 1}})
		p.send(t, map[string]any{"jsonrpc": "2.0", "method": "nurt/event", "params": map[string]int{"seq": 2}})
		p.send(t, result(m.ID, true))
	}()
	if _, err := c.Call(context.Background(), "run", nil); err != nil {
		t.Fatal(err)
	}
	mu.Lock()
	defer mu.Unlock()
	if len(seen) != 2 || seen[0] != `nurt/event{"seq":1}` || seen[1] != `nurt/event{"seq":2}` {
		t.Fatalf("notifications = %q", seen)
	}
}

func TestPeerRequestIsAnsweredWithMethodNotFound(t *testing.T) {
	_, p := newPair(t, nil)
	p.send(t, map[string]any{"jsonrpc": "2.0", "id": 41, "method": "client/ping"})
	m := p.next(t)
	if m.ID != 41 || m.Error == nil || m.Error.Code != CodeMethodNotFound {
		t.Fatalf("reply = %+v", m)
	}
}

func TestEndOfStreamFailsPendingCalls(t *testing.T) {
	c, p := newPair(t, nil)
	go func() {
		p.next(t)
		_ = p.fromSrv.Close()
	}()
	_, err := c.Call(context.Background(), "run", nil)
	if !errors.Is(err, ErrClosed) {
		t.Fatalf("error = %v, want ErrClosed", err)
	}
	<-c.Done()
	if _, err := c.NewCall(); !errors.Is(err, ErrClosed) {
		t.Fatalf("NewCall after end = %v, want ErrClosed", err)
	}
}

func TestInvalidMessageEndsConnection(t *testing.T) {
	c, p := newPair(t, nil)
	if err := p.w.Write([]byte("not json")); err != nil {
		t.Fatal(err)
	}
	select {
	case <-c.Done():
	case <-time.After(10 * time.Second):
		t.Fatal("connection still open after an invalid message")
	}
	if !errors.Is(c.Err(), ErrClosed) {
		t.Fatalf("error = %v, want ErrClosed", c.Err())
	}
}

func TestCallReturnsContextErrorAndIgnoresLateResponse(t *testing.T) {
	c, p := newPair(t, nil)
	ctx, cancel := context.WithCancel(context.Background())
	received := make(chan peerMessage, 1)
	go func() { received <- p.next(t) }()
	done := make(chan error, 1)
	go func() {
		_, err := c.Call(ctx, "run", nil)
		done <- err
	}()
	m := <-received
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatalf("error = %v, want context.Canceled", err)
	}
	p.send(t, result(m.ID, "late"))
	go func() {
		m := p.next(t)
		p.send(t, result(m.ID, "fresh"))
	}()
	res, err := c.Call(context.Background(), "run", nil)
	if err != nil || string(res) != `"fresh"` {
		t.Fatalf("result = %s, %v", res, err)
	}
}

func TestSendLimitRejectsLargeRequest(t *testing.T) {
	c, _ := newPair(t, nil)
	c.SetSendLimit(32)
	_, err := c.Call(context.Background(), "run", map[string]string{"program": "a long program text that exceeds the limit"})
	if !errors.Is(err, ErrMessageTooLarge) {
		t.Fatalf("error = %v, want ErrMessageTooLarge", err)
	}
}

func TestPostDiscardsResponse(t *testing.T) {
	c, p := newPair(t, nil)
	go func() {
		m := p.next(t)
		p.send(t, result(m.ID, "ignored"))
		m = p.next(t)
		p.send(t, result(m.ID, "answer"))
	}()
	if err := c.Post("cancel", map[string]int{"request_id": 1}); err != nil {
		t.Fatal(err)
	}
	res, err := c.Call(context.Background(), "run", nil)
	if err != nil || string(res) != `"answer"` {
		t.Fatalf("result = %s, %v", res, err)
	}
}

func TestCloseFailsPendingCalls(t *testing.T) {
	c, p := newPair(t, nil)
	read := make(chan struct{})
	go func() {
		p.next(t)
		close(read)
	}()
	call, err := c.NewCall()
	if err != nil {
		t.Fatal(err)
	}
	if err := c.Send(call, "run", nil); err != nil {
		t.Fatal(err)
	}
	<-read
	c.Close()
	<-call.Done()
	if _, err := call.Result(); !errors.Is(err, ErrClosed) {
		t.Fatalf("error = %v, want ErrClosed", err)
	}
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
