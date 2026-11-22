// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

//go:build nurt_cgo

package nurt

import (
	"context"
	"encoding/json"
	"errors"
	"math"
	"strconv"
	"sync"

	"github.com/DanacoOfficial/nurt/bindings/go/internal/capi"
)

// The library backend calls libnurt in process. Synchronous functions of
// the library cannot be interrupted, so ctx is checked before each of them;
// Run uses an asynchronous execution, which is cancelled when ctx ends.
type libraryBackend struct {
	mu     sync.RWMutex
	engine *capi.Engine

	execMu sync.Mutex
	execs  map[*libraryExec]struct{}
}

func newLibraryBackend(opts ClientOptions) (backend, error) {
	v := capi.ABIVersion()
	major, minor := int(v>>16), int(v&0xffff)
	if major != ABIMajor || minor < ABIMinor {
		return nil, newError(StatusABI, "libnurt implements interface "+strconv.Itoa(major)+"."+strconv.Itoa(minor)+
			"; this module needs "+strconv.Itoa(ABIMajor)+"."+strconv.Itoa(ABIMinor)+" or a later "+strconv.Itoa(ABIMajor)+".x", nil)
	}
	if err := capi.ABICheck(capi.HeaderVersion); err != nil {
		return nil, libraryError(err)
	}
	e, err := capi.Open(opts.Config)
	if err != nil {
		return nil, libraryError(err)
	}
	return &libraryBackend{engine: e, execs: make(map[*libraryExec]struct{})}, nil
}

func libraryError(err error) error {
	if err == nil {
		return nil
	}
	var ce *capi.CallError
	if !errors.As(err, &ce) {
		return newError(StatusInternal, err.Error(), err)
	}
	var rec struct {
		Message string          `json:"message"`
		Detail  json.RawMessage `json:"detail"`
	}
	if len(ce.Record) > 0 {
		_ = json.Unmarshal(ce.Record, &rec)
	}
	s := Status(ce.Status)
	msg := rec.Message
	if msg == "" {
		msg = "libnurt call failed with " + s.String()
	}
	e := newError(s, msg, nil)
	e.Detail = rec.Detail
	return e
}

func variantCode(v Variant) (int, error) {
	switch v {
	case VariantA:
		return 1, nil
	case VariantB:
		return 2, nil
	case VariantC:
		return 3, nil
	}
	return 0, newError(StatusArg, "unknown notation variant "+strconv.Quote(string(v))+"; use a, b or c", nil)
}

func (b *libraryBackend) use(ctx context.Context, f func(*capi.Engine) error) error {
	if err := ctx.Err(); err != nil {
		return contextError(err)
	}
	b.mu.RLock()
	defer b.mu.RUnlock()
	if b.engine == nil {
		return newError(StatusArg, "client is closed", nil)
	}
	return f(b.engine)
}

func (b *libraryBackend) close() error {
	b.mu.Lock()
	e := b.engine
	b.engine = nil
	b.mu.Unlock()
	if e == nil {
		return nil
	}
	b.execMu.Lock()
	live := make([]*libraryExec, 0, len(b.execs))
	for x := range b.execs {
		live = append(live, x)
	}
	b.execMu.Unlock()
	for _, x := range live {
		x.close()
	}
	e.Close()
	return nil
}

func (b *libraryBackend) startExec(ctx context.Context, env envelope, q *eventQueue) (*libraryExec, error) {
	req, err := json.Marshal(env)
	if err != nil {
		return nil, newError(StatusArg, "cannot encode the request: "+err.Error(), err)
	}
	var sink func([]byte)
	if q != nil {
		sink = func(raw []byte) {
			if ev, ok := decodeEvent(raw); ok {
				q.push(ev)
			}
		}
	}
	x := &libraryExec{b: b, q: q, done: make(chan struct{})}
	err = b.use(ctx, func(e *capi.Engine) error {
		cx, err := e.Start(req, sink)
		if err != nil {
			return libraryError(err)
		}
		x.x = cx
		b.execMu.Lock()
		b.execs[x] = struct{}{}
		b.execMu.Unlock()
		return nil
	})
	if err != nil {
		return nil, err
	}
	go x.await()
	return x, nil
}

func (b *libraryBackend) run(ctx context.Context, env envelope) (json.RawMessage, error) {
	x, err := b.startExec(ctx, env, nil)
	if err != nil {
		return nil, err
	}
	raw, err := x.wait(ctx)
	if raw == nil && ctx.Err() != nil {
		go x.close()
		return nil, err
	}
	x.close()
	return raw, err
}

func (b *libraryBackend) start(ctx context.Context, env envelope, q *eventQueue) (execHandle, error) {
	return b.startExec(ctx, env, q)
}

func (b *libraryBackend) check(ctx context.Context, env envelope) (*CheckResult, error) {
	req, err := json.Marshal(env)
	if err != nil {
		return nil, newError(StatusArg, "cannot encode the request: "+err.Error(), err)
	}
	var res CheckResult
	err = b.use(ctx, func(e *capi.Engine) error {
		bound, diag, err := e.Check(req)
		res = CheckResult{Bound: bound, Diagnostics: diag}
		return libraryError(err)
	})
	if err != nil {
		return nil, err
	}
	return &res, nil
}

func (b *libraryBackend) parse(ctx context.Context, program string, v Variant) (*ParseResult, error) {
	code, err := variantCode(v)
	if err != nil {
		return nil, err
	}
	var res ParseResult
	err = b.use(ctx, func(e *capi.Engine) error {
		ast, diag, err := e.Parse(program, code)
		res = ParseResult{AST: ast, Diagnostics: diag}
		return libraryError(err)
	})
	if err != nil {
		return nil, err
	}
	return &res, nil
}

func (b *libraryBackend) print(ctx context.Context, ast json.RawMessage, v Variant) (string, error) {
	code, err := variantCode(v)
	if err != nil {
		return "", err
	}
	var text []byte
	err = b.use(ctx, func(e *capi.Engine) error {
		var err error
		text, err = e.Print(ast, code)
		return libraryError(err)
	})
	return string(text), err
}

func (b *libraryBackend) correct(ctx context.Context, program string, v Variant) (json.RawMessage, error) {
	code, err := variantCode(v)
	if err != nil {
		return nil, err
	}
	return b.query(ctx, func(e *capi.Engine) ([]byte, error) { return e.Correct(program, code) })
}

func (b *libraryBackend) page(ctx context.Context, cursor string, limit int) (json.RawMessage, error) {
	if uint64(limit) > math.MaxUint32 {
		return nil, newError(StatusArg, "page limit exceeds "+strconv.FormatUint(math.MaxUint32, 10), nil)
	}
	return b.query(ctx, func(e *capi.Engine) ([]byte, error) { return e.Page(cursor, uint32(limit)) })
}

func (b *libraryBackend) describe(ctx context.Context, q DescribeQuery) (string, error) {
	var what []byte
	if q != (DescribeQuery{}) {
		var err error
		if what, err = json.Marshal(q); err != nil {
			return "", newError(StatusArg, "cannot encode the query: "+err.Error(), err)
		}
	}
	text, err := b.query(ctx, func(e *capi.Engine) ([]byte, error) { return e.Describe(what) })
	return string(text), err
}

func (b *libraryBackend) help(ctx context.Context, topic string) (string, error) {
	text, err := b.query(ctx, func(e *capi.Engine) ([]byte, error) { return e.Help(topic) })
	return string(text), err
}

func (b *libraryBackend) catalog(ctx context.Context, what json.RawMessage) (json.RawMessage, error) {
	return b.query(ctx, func(e *capi.Engine) ([]byte, error) { return e.Catalog(what) })
}

func (b *libraryBackend) schema(ctx context.Context, which string) (json.RawMessage, error) {
	return b.query(ctx, func(*capi.Engine) ([]byte, error) { return capi.Schema(which) })
}

func (b *libraryBackend) history(ctx context.Context, query json.RawMessage) (json.RawMessage, error) {
	return b.query(ctx, func(e *capi.Engine) ([]byte, error) { return e.History(query) })
}

func (b *libraryBackend) space(ctx context.Context, volume string) (json.RawMessage, error) {
	return b.query(ctx, func(e *capi.Engine) ([]byte, error) { return e.Space(volume) })
}

func (b *libraryBackend) info(ctx context.Context) (json.RawMessage, error) {
	return b.query(ctx, func(*capi.Engine) ([]byte, error) { return capi.Info() })
}

func (b *libraryBackend) query(ctx context.Context, f func(*capi.Engine) ([]byte, error)) (json.RawMessage, error) {
	var out []byte
	err := b.use(ctx, func(e *capi.Engine) error {
		var err error
		out, err = f(e)
		return libraryError(err)
	})
	if err != nil {
		return nil, err
	}
	return out, nil
}

// One goroutine per execution waits for its completion, so that Wait with
// a context never leaves a library call behind and the handle is released
// exactly once.
type libraryExec struct {
	b    *libraryBackend
	x    *capi.Exec
	q    *eventQueue
	done chan struct{}
	resp []byte
	err  error

	mu    sync.Mutex
	freed bool
}

func (x *libraryExec) await() {
	x.resp, x.err = x.x.Wait(capi.WaitForever)
	close(x.done)
	if x.q != nil {
		x.q.close()
	}
}

func (x *libraryExec) wait(ctx context.Context) (json.RawMessage, error) {
	select {
	case <-x.done:
		return x.resp, libraryError(x.err)
	case <-ctx.Done():
		return nil, contextError(ctx.Err())
	}
}

func (x *libraryExec) cancel(context.Context) error {
	x.mu.Lock()
	defer x.mu.Unlock()
	if x.freed {
		return nil
	}
	return libraryError(x.x.Cancel())
}

func (x *libraryExec) close() {
	x.mu.Lock()
	defer x.mu.Unlock()
	if x.freed {
		return
	}
	select {
	case <-x.done:
	default:
		_ = x.x.Cancel()
		<-x.done
	}
	x.x.Free()
	x.freed = true
	x.b.execMu.Lock()
	delete(x.b.execs, x)
	x.b.execMu.Unlock()
}
