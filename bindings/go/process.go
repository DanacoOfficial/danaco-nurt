// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/DanacoOfficial/nurt/bindings/go/internal/rpc"
)

const (
	protocolName = "nurt-rpc/1"
	clientName   = "nurt-go"
	eventMethod  = "nurt/event"

	// ABIMajor and ABIMinor are the interface version this module needs:
	// the major version must match and the minor version must not be
	// lower.
	ABIMajor = 1
	ABIMinor = 0

	// Largest message accepted from nurt serve. Responses with many
	// records are paged, so this bound is far above normal traffic.
	maxIncomingMessage = 256 << 20

	shutdownTimeout = 5 * time.Second
	exitGrace       = 5 * time.Second
	stderrTailBytes = 4096

	// JSON-RPC error codes below appErrorFirst down to appErrorLast carry a
	// call status: the status is appErrorFirst minus the code, so -32003 is
	// StatusArg.
	appErrorFirst = -32000
	appErrorLast  = -32099
)

// Interface version as carried by nurt-rpc/1: (major << 16) | minor.
const (
	abiMajorShift = 16
	abiMinorMask  = 1<<abiMajorShift - 1
	abiRequired   = ABIMajor<<abiMajorShift | ABIMinor
)

type initializeParams struct {
	Protocol        string          `json:"protocol"`
	Client          clientInfo      `json:"client"`
	ABI             uint32          `json:"abi"`
	Language        string          `json:"language,omitempty"`
	Session         string          `json:"session,omitempty"`
	ConfigOverrides json.RawMessage `json:"config_overrides,omitempty"`
	Capabilities    capabilities    `json:"capabilities"`
}

type clientInfo struct {
	Name    string `json:"name"`
	Version string `json:"version"`
}

type capabilities struct {
	Events bool `json:"events"`
}

type initializeResult struct {
	Protocol string       `json:"protocol"`
	ABI      *uint32      `json:"abi"`
	Limits   serverLimits `json:"limits"`
}

type serverLimits struct {
	MaxInflight     int `json:"max_inflight"`
	MaxMessageBytes int `json:"max_message_bytes"`
}

type cancelParams struct {
	RequestID int64 `json:"request_id"`
}

type programParams struct {
	Program string  `json:"program"`
	Variant Variant `json:"variant"`
}

type printParams struct {
	AST     json.RawMessage `json:"ast"`
	Variant Variant         `json:"variant"`
}

type pageParams struct {
	Cursor string `json:"cursor"`
	Limit  int    `json:"limit,omitempty"`
}

type helpParams struct {
	Topic string `json:"topic,omitempty"`
}

type schemaParams struct {
	Name string `json:"name"`
}

type describeParams struct {
	Subject  string `json:"subject,omitempty"`
	Language string `json:"language,omitempty"`
}

type describeResult struct {
	Description json.RawMessage `json:"description"`
}

type spaceParams struct {
	Volume string `json:"volume,omitempty"`
}

type textResult struct {
	Text string `json:"text"`
}

// The process backend supervises one nurt serve process at a time. A
// process that ends unexpectedly fails the calls in flight and is replaced
// on the next call.
type processBackend struct {
	exe     string
	args    []string
	env     []string
	framing rpc.Framing
	init    initializeParams

	mu     sync.Mutex
	cur    *serverProcess
	closed bool
}

func newProcessBackend(ctx context.Context, opts ClientOptions, h host) (*processBackend, error) {
	exe, err := findExecutable(opts.Executable, h)
	if err != nil {
		return nil, err
	}
	framing := rpc.NDJSON
	if opts.Framing == FramingContentLength {
		framing = rpc.ContentLength
	}
	args := opts.Args
	if args == nil {
		args = []string{"serve", "--stdio"}
		if framing == rpc.ContentLength {
			args = append(args, "--framing", "content-length")
		}
	}
	b := &processBackend{
		exe:     exe,
		args:    args,
		env:     opts.Env,
		framing: framing,
		init: initializeParams{
			Protocol:        protocolName,
			Client:          clientInfo{Name: clientName, Version: Version},
			ABI:             abiRequired,
			Language:        opts.Language,
			Session:         opts.Session,
			ConfigOverrides: opts.Config,
			Capabilities:    capabilities{Events: true},
		},
	}
	if _, err := b.ensure(ctx); err != nil {
		return nil, err
	}
	return b, nil
}

func (b *processBackend) ensure(ctx context.Context) (*serverProcess, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return nil, newError(StatusArg, "client is closed", nil)
	}
	if b.cur != nil && !b.cur.ended() {
		return b.cur, nil
	}
	p, err := b.spawn(ctx)
	if err != nil {
		return nil, err
	}
	b.cur = p
	return p, nil
}

func (b *processBackend) spawn(ctx context.Context) (*serverProcess, error) {
	cmd := exec.Command(b.exe, b.args...)
	cmd.Env = append(os.Environ(), b.env...)
	cmd.WaitDelay = exitGrace
	stdin, err := cmd.StdinPipe()
	if err != nil {
		return nil, newError(StatusIO, "cannot create a pipe for nurt: "+err.Error(), err)
	}
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, newError(StatusIO, "cannot create a pipe for nurt: "+err.Error(), err)
	}
	tail := &tailBuffer{max: stderrTailBytes}
	cmd.Stderr = tail
	if err := cmd.Start(); err != nil {
		return nil, newError(StatusIO, "cannot start "+b.exe+": "+err.Error(), err)
	}
	p := &serverProcess{
		cmd:    cmd,
		stdin:  stdin,
		stderr: tail,
		exited: make(chan struct{}),
		sinks:  make(map[int64]*eventQueue),
	}
	p.conn = rpc.NewConn(stdout, stdin, b.framing, maxIncomingMessage, p.dispatch)
	go p.supervise()

	raw, err := p.conn.Call(ctx, "initialize", b.init)
	if err != nil {
		p.kill()
		return nil, p.mapError(ctx, err)
	}
	var res initializeResult
	if err := json.Unmarshal(raw, &res); err != nil {
		p.kill()
		return nil, newError(StatusInternal, "invalid initialize result from nurt: "+err.Error(), err)
	}
	if res.Protocol != protocolName {
		p.kill()
		return nil, newError(StatusABI, "nurt serve speaks protocol "+strconv.Quote(res.Protocol)+"; this module needs "+protocolName, nil)
	}
	if res.ABI != nil && (*res.ABI>>abiMajorShift != ABIMajor || *res.ABI&abiMinorMask < ABIMinor) {
		p.kill()
		return nil, newError(StatusABI, "nurt implements interface "+strconv.Itoa(int(*res.ABI>>abiMajorShift))+"."+strconv.Itoa(int(*res.ABI&abiMinorMask))+
			"; this module needs "+strconv.Itoa(ABIMajor)+"."+strconv.Itoa(ABIMinor)+" or a later "+strconv.Itoa(ABIMajor)+".x", nil)
	}
	if res.Limits.MaxMessageBytes > 0 {
		p.conn.SetSendLimit(res.Limits.MaxMessageBytes)
	}
	if res.Limits.MaxInflight > 0 {
		p.slots = make(chan struct{}, res.Limits.MaxInflight)
	}
	return p, nil
}

func (b *processBackend) close() error {
	b.mu.Lock()
	b.closed = true
	p := b.cur
	b.cur = nil
	b.mu.Unlock()
	if p == nil {
		return nil
	}
	if !p.ended() {
		ctx, cancel := context.WithTimeout(context.Background(), shutdownTimeout)
		_, _ = p.conn.Call(ctx, "shutdown", struct{}{})
		cancel()
	}
	_ = p.stdin.Close()
	select {
	case <-p.exited:
	case <-time.After(exitGrace):
		p.kill()
		<-p.exited
	}
	return nil
}

func (b *processBackend) call(ctx context.Context, method string, params any) (json.RawMessage, error) {
	if err := ctx.Err(); err != nil {
		return nil, contextError(err)
	}
	p, err := b.ensure(ctx)
	if err != nil {
		return nil, err
	}
	call, err := p.begin(ctx, method, params, nil)
	if err != nil {
		return nil, err
	}
	select {
	case <-call.Done():
		p.release()
		res, err := call.Result()
		if err != nil {
			return nil, p.mapError(ctx, err)
		}
		return res, nil
	case <-ctx.Done():
		p.conn.Abandon(call)
		p.release()
		_ = p.conn.Post("cancel", cancelParams{RequestID: call.ID})
		return nil, contextError(ctx.Err())
	}
}

func (b *processBackend) callInto(ctx context.Context, method string, params any, out any) error {
	raw, err := b.call(ctx, method, params)
	if err != nil {
		return err
	}
	if err := json.Unmarshal(raw, out); err != nil {
		return newError(StatusInternal, "invalid "+method+" result from nurt: "+err.Error(), err)
	}
	return nil
}

func (b *processBackend) callText(ctx context.Context, method string, params any) (string, error) {
	var res textResult
	if err := b.callInto(ctx, method, params, &res); err != nil {
		return "", err
	}
	return res.Text, nil
}

func (b *processBackend) run(ctx context.Context, env envelope) (json.RawMessage, error) {
	return b.call(ctx, "run", env)
}

func (b *processBackend) start(ctx context.Context, env envelope, q *eventQueue) (execHandle, error) {
	if err := ctx.Err(); err != nil {
		return nil, contextError(err)
	}
	p, err := b.ensure(ctx)
	if err != nil {
		return nil, err
	}
	call, err := p.begin(ctx, "run", env, q)
	if err != nil {
		return nil, err
	}
	x := &processExec{p: p, call: call, closed: make(chan struct{})}
	go func() {
		select {
		case <-call.Done():
		case <-x.closed:
		}
		p.removeSink(call.ID)
		p.release()
		q.close()
	}()
	return x, nil
}

func (b *processBackend) check(ctx context.Context, env envelope) (*CheckResult, error) {
	var res CheckResult
	if err := b.callInto(ctx, "check", env, &res); err != nil {
		return nil, err
	}
	return &res, nil
}

func (b *processBackend) parse(ctx context.Context, program string, v Variant) (*ParseResult, error) {
	var res ParseResult
	if err := b.callInto(ctx, "parse", programParams{Program: program, Variant: v}, &res); err != nil {
		return nil, err
	}
	return &res, nil
}

func (b *processBackend) print(ctx context.Context, ast json.RawMessage, v Variant) (string, error) {
	return b.callText(ctx, "print", printParams{AST: ast, Variant: v})
}

func (b *processBackend) correct(ctx context.Context, program string, v Variant) (json.RawMessage, error) {
	return b.call(ctx, "correct", programParams{Program: program, Variant: v})
}

func (b *processBackend) page(ctx context.Context, cursor string, limit int) (json.RawMessage, error) {
	return b.call(ctx, "page", pageParams{Cursor: cursor, Limit: limit})
}

func (b *processBackend) describe(ctx context.Context, q DescribeQuery) (string, error) {
	subject := q.Name
	if subject == "" {
		subject = q.Layer
	}
	var res describeResult
	if err := b.callInto(ctx, "describe", describeParams{Subject: subject, Language: q.Language}, &res); err != nil {
		return "", err
	}
	var text string
	if json.Unmarshal(res.Description, &text) == nil {
		return text, nil
	}
	return string(res.Description), nil
}

func (b *processBackend) help(ctx context.Context, topic string) (string, error) {
	return b.callText(ctx, "help", helpParams{Topic: topic})
}

func (b *processBackend) catalog(ctx context.Context, what json.RawMessage) (json.RawMessage, error) {
	return b.call(ctx, "catalog", objectOrEmpty(what))
}

func (b *processBackend) schema(ctx context.Context, which string) (json.RawMessage, error) {
	return b.call(ctx, "schema", schemaParams{Name: which})
}

func (b *processBackend) history(ctx context.Context, query json.RawMessage) (json.RawMessage, error) {
	return b.call(ctx, "history", objectOrEmpty(query))
}

func (b *processBackend) space(ctx context.Context, volume string) (json.RawMessage, error) {
	return b.call(ctx, "space", spaceParams{Volume: volume})
}

func (b *processBackend) info(ctx context.Context) (json.RawMessage, error) {
	return b.call(ctx, "info", struct{}{})
}

// JSON-RPC parameters must be structured, so an absent selector is sent as
// an empty object.
func objectOrEmpty(raw json.RawMessage) json.RawMessage {
	if len(raw) == 0 {
		return json.RawMessage("{}")
	}
	return raw
}

type serverProcess struct {
	cmd    *exec.Cmd
	stdin  io.WriteCloser
	stderr *tailBuffer
	conn   *rpc.Conn
	slots  chan struct{}

	exited  chan struct{}
	exitErr error

	sinksMu sync.Mutex
	sinks   map[int64]*eventQueue
}

func (p *serverProcess) ended() bool {
	select {
	case <-p.conn.Done():
		return true
	default:
		return false
	}
}

func (p *serverProcess) kill() {
	_ = p.cmd.Process.Kill()
}

// The process is reaped only after the reader has reached the end of its
// output, because Wait closes the pipe. An output that is not a valid
// message stream means the process cannot be trusted, so it is killed at
// once; otherwise it gets exitGrace to finish writing its journal.
func (p *serverProcess) supervise() {
	<-p.conn.Done()
	if err := p.conn.Err(); err != nil && !errors.Is(err, io.EOF) {
		p.kill()
	}
	waited := make(chan error, 1)
	go func() { waited <- p.cmd.Wait() }()
	select {
	case err := <-waited:
		p.exitErr = err
	case <-time.After(exitGrace):
		p.kill()
		p.exitErr = <-waited
	}
	close(p.exited)
}

func (p *serverProcess) begin(ctx context.Context, method string, params any, q *eventQueue) (*rpc.Call, error) {
	if p.slots != nil {
		select {
		case p.slots <- struct{}{}:
		case <-ctx.Done():
			return nil, contextError(ctx.Err())
		}
	}
	call, err := p.conn.NewCall()
	if err != nil {
		p.release()
		return nil, p.mapError(ctx, err)
	}
	if q != nil {
		p.sinksMu.Lock()
		p.sinks[call.ID] = q
		p.sinksMu.Unlock()
	}
	if err := p.conn.Send(call, method, params); err != nil {
		p.removeSink(call.ID)
		p.release()
		return nil, p.mapError(ctx, err)
	}
	return call, nil
}

func (p *serverProcess) release() {
	if p.slots != nil {
		<-p.slots
	}
}

func (p *serverProcess) removeSink(id int64) {
	p.sinksMu.Lock()
	delete(p.sinks, id)
	p.sinksMu.Unlock()
}

func (p *serverProcess) dispatch(method string, params json.RawMessage) {
	if method != eventMethod {
		return
	}
	var head struct {
		RequestID json.RawMessage `json:"request"`
	}
	if err := json.Unmarshal(params, &head); err != nil {
		return
	}
	id, ok := parseRequestID(head.RequestID)
	if !ok {
		return
	}
	p.sinksMu.Lock()
	q := p.sinks[id]
	p.sinksMu.Unlock()
	if q == nil {
		return
	}
	if ev, ok := decodeEvent(params); ok {
		q.push(ev)
	}
}

// Request identifiers are sent as numbers; events carry them as decimal
// text, and a number is accepted as well.
func parseRequestID(raw json.RawMessage) (int64, bool) {
	if len(raw) == 0 {
		return 0, false
	}
	var n int64
	if err := json.Unmarshal(raw, &n); err == nil {
		return n, true
	}
	var s string
	if err := json.Unmarshal(raw, &s); err != nil {
		return 0, false
	}
	n, err := strconv.ParseInt(s, 10, 64)
	return n, err == nil
}

func (p *serverProcess) mapError(ctx context.Context, err error) error {
	var rpcErr *rpc.Error
	switch {
	case ctx.Err() != nil && errors.Is(err, ctx.Err()):
		return contextError(ctx.Err())
	case errors.As(err, &rpcErr):
		return statusFromRPC(rpcErr)
	case errors.Is(err, rpc.ErrMessageTooLarge):
		return newError(StatusArg, err.Error(), err)
	case errors.Is(err, rpc.ErrClosed):
		return p.exitError(err)
	default:
		return newError(StatusInternal, err.Error(), err)
	}
}

func (p *serverProcess) exitError(cause error) *Error {
	var b strings.Builder
	b.WriteString("nurt process ended unexpectedly")
	if err := p.conn.Err(); err != nil && !errors.Is(err, io.EOF) {
		b.WriteString(": ")
		b.WriteString(err.Error())
	}
	select {
	case <-p.exited:
		if p.exitErr != nil {
			b.WriteString(": ")
			b.WriteString(p.exitErr.Error())
		}
	case <-time.After(exitGrace):
	}
	if tail := strings.TrimSpace(p.stderr.String()); tail != "" {
		b.WriteString("; stderr: ")
		b.WriteString(tail)
	}
	return newError(StatusInternal, b.String(), cause)
}

func statusFromRPC(e *rpc.Error) *Error {
	var s Status
	switch {
	case e.Code < appErrorFirst && e.Code >= appErrorLast:
		s = Status(appErrorFirst - e.Code)
	case e.Code == rpc.CodeInvalidParams:
		s = StatusArg
	case e.Code == rpc.CodeMethodNotFound:
		s = StatusABI
	default:
		s = StatusInternal
	}
	err := newError(s, e.Message, e)
	err.Detail = e.Data
	return err
}

type processExec struct {
	p    *serverProcess
	call *rpc.Call

	mu              sync.Mutex
	cancelRequested bool

	closed    chan struct{}
	closeOnce sync.Once
}

func (x *processExec) wait(ctx context.Context) (json.RawMessage, error) {
	select {
	case <-x.call.Done():
	case <-x.closed:
		return nil, newError(StatusArg, "execution is closed", nil)
	case <-ctx.Done():
		return nil, contextError(ctx.Err())
	}
	res, err := x.call.Result()
	if err != nil {
		return nil, x.p.mapError(ctx, err)
	}
	x.mu.Lock()
	cancelled := x.cancelRequested
	x.mu.Unlock()
	if cancelled {
		return res, newError(StatusCancelled, "execution was cancelled", nil)
	}
	return res, nil
}

func (x *processExec) cancel(ctx context.Context) error {
	select {
	case <-x.call.Done():
		return nil
	case <-x.closed:
		return nil
	default:
	}
	x.mu.Lock()
	x.cancelRequested = true
	x.mu.Unlock()
	if _, err := x.p.conn.Call(ctx, "cancel", cancelParams{RequestID: x.call.ID}); err != nil {
		return x.p.mapError(ctx, err)
	}
	return nil
}

func (x *processExec) close() {
	x.closeOnce.Do(func() {
		select {
		case <-x.call.Done():
		default:
			x.p.conn.Abandon(x.call)
			_ = x.p.conn.Post("cancel", cancelParams{RequestID: x.call.ID})
		}
		close(x.closed)
	})
}

// The tail of the standard error of nurt is kept for error messages; logs
// of nurt serve go to standard error, never to the protocol stream.
type tailBuffer struct {
	mu  sync.Mutex
	max int
	buf []byte
}

func (t *tailBuffer) Write(b []byte) (int, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	t.buf = append(t.buf, b...)
	if over := len(t.buf) - t.max; over > 0 {
		t.buf = append(t.buf[:0], t.buf[over:]...)
	}
	return len(b), nil
}

func (t *tailBuffer) String() string {
	t.mu.Lock()
	defer t.mu.Unlock()
	return string(t.buf)
}
