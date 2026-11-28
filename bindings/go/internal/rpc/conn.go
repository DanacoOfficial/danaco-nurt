// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/go/internal/rpc/conn.go
// Responsibility:   Source file bindings/go/internal/rpc/conn.go.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package rpc

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"sync"
)

// Version is the value of the jsonrpc member of every message.
const Version = "2.0"

// Error codes defined by JSON-RPC 2.0.
const (
	CodeParseError     = -32700
	CodeInvalidRequest = -32600
	CodeMethodNotFound = -32601
	CodeInvalidParams  = -32602
	CodeInternalError  = -32603
)

// ErrClosed is wrapped by every error caused by the end of the connection.
var ErrClosed = errors.New("connection closed")

// Error is a JSON-RPC error object returned by the peer.
type Error struct {
	Code    int             `json:"code"`
	Message string          `json:"message"`
	Data    json.RawMessage `json:"data,omitempty"`
}

func (e *Error) Error() string {
	return fmt.Sprintf("json-rpc error %d: %s", e.Code, e.Message)
}

type closedError struct {
	cause error
}

func (e *closedError) Error() string {
	if e.cause == nil || errors.Is(e.cause, io.EOF) {
		return ErrClosed.Error()
	}
	return ErrClosed.Error() + ": " + e.cause.Error()
}

func (e *closedError) Unwrap() []error {
	if e.cause == nil {
		return []error{ErrClosed}
	}
	return []error{ErrClosed, e.cause}
}

type outgoing struct {
	JSONRPC string `json:"jsonrpc"`
	ID      *int64 `json:"id,omitempty"`
	Method  string `json:"method"`
	Params  any    `json:"params,omitempty"`
}

type errorReply struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Error   *Error          `json:"error"`
}

type incoming struct {
	JSONRPC string          `json:"jsonrpc"`
	ID      json.RawMessage `json:"id"`
	Method  string          `json:"method"`
	Params  json.RawMessage `json:"params"`
	Result  json.RawMessage `json:"result"`
	Error   *Error          `json:"error"`
}

// Call is one request awaiting its response.
type Call struct {
	// ID is the request identifier sent to the peer.
	ID     int64
	done   chan struct{}
	result json.RawMessage
	err    error
}

// Done is closed when the response has arrived or the connection has ended.
func (c *Call) Done() <-chan struct{} { return c.done }

// Result returns the result member of the response, or an error: *Error for
// an error response, an error wrapping ErrClosed when the connection ended
// first. It must be called after Done is closed.
func (c *Call) Result() (json.RawMessage, error) { return c.result, c.err }

// NotifyFunc receives a notification. It is called from the reading
// goroutine, in the order of arrival, and must not block.
type NotifyFunc func(method string, params json.RawMessage)

// Conn is a JSON-RPC 2.0 client connection over a pair of byte streams.
// Several requests may be in flight at the same time; responses are matched
// to requests by identifier in any order. Conn is safe for concurrent use.
type Conn struct {
	reader *Reader
	writer *Writer
	notify NotifyFunc

	mu        sync.Mutex
	nextID    int64
	pending   map[int64]*Call
	err       error
	sendLimit int

	done chan struct{}
}

// NewConn starts a connection that reads messages of at most maxMessage
// bytes from r and writes requests to w. notify may be nil.
func NewConn(r io.Reader, w io.Writer, f Framing, maxMessage int, notify NotifyFunc) *Conn {
	c := &Conn{
		reader:  NewReader(r, f, maxMessage),
		writer:  NewWriter(w, f),
		notify:  notify,
		pending: make(map[int64]*Call),
		done:    make(chan struct{}),
	}
	go c.readLoop()
	return c
}

// SetSendLimit sets the largest request, in bytes, accepted by the peer; 0
// removes the limit.
func (c *Conn) SetSendLimit(n int) {
	c.mu.Lock()
	c.sendLimit = n
	c.mu.Unlock()
}

// Done is closed when the connection has ended.
func (c *Conn) Done() <-chan struct{} { return c.done }

// Err returns the error that ended the connection, or nil while it is open.
func (c *Conn) Err() error {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.err
}

// NewCall allocates a request identifier and registers the call, so that
// notifications referring to the identifier can be routed before the
// request is sent.
func (c *Conn) NewCall() (*Call, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.err != nil {
		return nil, c.err
	}
	c.nextID++
	call := &Call{ID: c.nextID, done: make(chan struct{})}
	c.pending[call.ID] = call
	return call, nil
}

// Send writes the request of a call registered by NewCall. On failure the
// call is removed and completed with the returned error.
func (c *Conn) Send(call *Call, method string, params any) error {
	err := c.write(&call.ID, method, params)
	if err != nil {
		c.complete(call.ID, nil, err)
	}
	return err
}

// Post sends a request whose response is discarded.
func (c *Conn) Post(method string, params any) error {
	call, err := c.NewCall()
	if err != nil {
		return err
	}
	c.Abandon(call)
	return c.write(&call.ID, method, params)
}

// Call sends a request and waits for its response. When ctx ends first the
// call is abandoned and the context error is returned.
func (c *Conn) Call(ctx context.Context, method string, params any) (json.RawMessage, error) {
	call, err := c.NewCall()
	if err != nil {
		return nil, err
	}
	if err := c.Send(call, method, params); err != nil {
		return nil, err
	}
	select {
	case <-call.Done():
		return call.Result()
	case <-ctx.Done():
		c.Abandon(call)
		return nil, ctx.Err()
	}
}

// Abandon removes a call; a later response to it is ignored.
func (c *Conn) Abandon(call *Call) {
	c.mu.Lock()
	delete(c.pending, call.ID)
	c.mu.Unlock()
}

// Close ends the connection with ErrClosed and completes every pending
// call. It does not close the underlying streams.
func (c *Conn) Close() { c.fail(nil) }

func (c *Conn) write(id *int64, method string, params any) error {
	msg, err := json.Marshal(outgoing{JSONRPC: Version, ID: id, Method: method, Params: params})
	if err != nil {
		return fmt.Errorf("encode %s request: %w", method, err)
	}
	c.mu.Lock()
	limit, closed := c.sendLimit, c.err
	c.mu.Unlock()
	if closed != nil {
		return closed
	}
	if limit > 0 && len(msg) > limit {
		return fmt.Errorf("%w: %s request of %d bytes, limit %d", ErrMessageTooLarge, method, len(msg), limit)
	}
	if err := c.writer.Write(msg); err != nil {
		return &closedError{cause: err}
	}
	return nil
}

func (c *Conn) readLoop() {
	for {
		msg, err := c.reader.Read()
		if err != nil {
			c.fail(err)
			return
		}
		var in incoming
		if err := json.Unmarshal(msg, &in); err != nil {
			c.fail(fmt.Errorf("invalid message from peer: %w", err))
			return
		}
		if in.JSONRPC != Version {
			c.fail(fmt.Errorf("invalid message from peer: jsonrpc member is %q", in.JSONRPC))
			return
		}
		c.dispatch(&in)
	}
}

func (c *Conn) dispatch(in *incoming) {
	hasID := len(in.ID) > 0 && string(in.ID) != "null"
	switch {
	case in.Method != "" && !hasID:
		if c.notify != nil {
			c.notify(in.Method, in.Params)
		}
	case in.Method != "":
		reply := errorReply{
			JSONRPC: Version,
			ID:      in.ID,
			Error:   &Error{Code: CodeMethodNotFound, Message: "method not found: " + in.Method},
		}
		if msg, err := json.Marshal(reply); err == nil {
			_ = c.writer.Write(msg)
		}
	case hasID:
		var id int64
		if err := json.Unmarshal(in.ID, &id); err != nil {
			return
		}
		if in.Error != nil {
			c.complete(id, nil, in.Error)
			return
		}
		result := in.Result
		if result == nil {
			result = json.RawMessage("null")
		}
		c.complete(id, result, nil)
	}
}

func (c *Conn) complete(id int64, result json.RawMessage, err error) {
	c.mu.Lock()
	call, ok := c.pending[id]
	delete(c.pending, id)
	c.mu.Unlock()
	if !ok {
		return
	}
	call.result, call.err = result, err
	close(call.done)
}

func (c *Conn) fail(cause error) {
	c.mu.Lock()
	if c.err != nil {
		c.mu.Unlock()
		return
	}
	c.err = &closedError{cause: cause}
	pending := c.pending
	c.pending = make(map[int64]*Call)
	err := c.err
	c.mu.Unlock()
	for _, call := range pending {
		call.err = err
		close(call.done)
	}
	close(c.done)
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
