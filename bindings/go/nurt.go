// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"context"
	"encoding/json"
	"sync/atomic"
)

// Version is the version of this module. It equals the version of the nurt
// release the module is published with.
const Version = "0.1.1"

// Backend selects how the client reaches Danaco Nurt.
type Backend string

// Backends.
const (
	// BackendAuto selects BackendProcess.
	BackendAuto Backend = "auto"
	// BackendProcess runs nurt serve --stdio as a child process and talks
	// to it with the JSON-RPC 2.0 protocol nurt-rpc/1. It needs no C
	// compiler.
	BackendProcess Backend = "process"
	// BackendLibrary calls libnurt in process through cgo. It is available
	// only when the module is built with the nurt_cgo build tag.
	BackendLibrary Backend = "library"
)

// Framing selects the message framing of the process backend.
type Framing int

// Framings.
const (
	// FramingNDJSON sends one JSON message per line.
	FramingNDJSON Framing = iota
	// FramingContentLength prefixes each message with a Content-Length
	// header.
	FramingContentLength
)

// ClientOptions configure a Client. The zero value selects the process
// backend and finds nurt through NURT_EXECUTABLE or PATH.
type ClientOptions struct {
	// Backend selects the backend; empty means BackendAuto.
	Backend Backend
	// Executable is the path of the nurt executable used by the process
	// backend. When empty, NURT_EXECUTABLE and then PATH are searched.
	Executable string
	// Args are the arguments passed to Executable; nil means
	// "serve --stdio", followed by the framing option when
	// FramingContentLength is selected.
	Args []string
	// Env holds additional environment variables of the child process in
	// the form "KEY=value", for example NURT_CONFIG or NURT_STATE_DIR.
	Env []string
	// Framing selects the message framing of the process backend.
	Framing Framing
	// Config is a configuration object nurt-config/1 applied on top of the
	// configuration files.
	Config json.RawMessage
	// Session is the default session identifier of requests.
	Session string
	// Language is the default language of messages and help ("en" or
	// "pl").
	Language string
}

// Client is a connection to Danaco Nurt. Its methods are safe for
// concurrent use. A Client must be closed with Close.
type Client struct {
	opts   ClientOptions
	b      backend
	closed atomic.Bool
}

// Each backend implements the calls of Client; arguments are validated and
// defaults applied by Client before a backend is called.
type backend interface {
	run(ctx context.Context, env envelope) (json.RawMessage, error)
	start(ctx context.Context, env envelope, q *eventQueue) (execHandle, error)
	check(ctx context.Context, env envelope) (*CheckResult, error)
	parse(ctx context.Context, program string, v Variant) (*ParseResult, error)
	print(ctx context.Context, ast json.RawMessage, v Variant) (string, error)
	correct(ctx context.Context, program string, v Variant) (json.RawMessage, error)
	page(ctx context.Context, cursor string, limit int) (json.RawMessage, error)
	describe(ctx context.Context, q DescribeQuery) (string, error)
	help(ctx context.Context, topic string) (string, error)
	catalog(ctx context.Context, what json.RawMessage) (json.RawMessage, error)
	schema(ctx context.Context, which string) (json.RawMessage, error)
	history(ctx context.Context, query json.RawMessage) (json.RawMessage, error)
	space(ctx context.Context, volume string) (json.RawMessage, error)
	info(ctx context.Context) (json.RawMessage, error)
	close() error
}

// New creates a client. With the process backend it starts nurt serve and
// negotiates the protocol version; ctx bounds the start only.
func New(ctx context.Context, opts ClientOptions) (*Client, error) {
	var (
		b   backend
		err error
	)
	switch opts.Backend {
	case "", BackendAuto, BackendProcess:
		b, err = newProcessBackend(ctx, opts, systemHost)
	case BackendLibrary:
		b, err = newLibraryBackend(opts)
	default:
		err = newError(StatusArg, "unknown backend "+string(opts.Backend)+"; use auto, process or library", nil)
	}
	if err != nil {
		return nil, err
	}
	return &Client{opts: opts, b: b}, nil
}

func (c *Client) usable() error {
	if c.closed.Load() {
		return newError(StatusArg, "client is closed", nil)
	}
	return nil
}

// Run runs a request and waits for its response. A language error is
// returned in the response; with req.FailOnLanguageError it is also
// returned as a *LanguageError. When ctx ends first, the request is
// cancelled and Run returns an *Error with StatusTimeout or
// StatusCancelled.
func (c *Client) Run(ctx context.Context, req Request) (*Response, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	raw, err := c.b.run(ctx, c.envelope(req))
	if err != nil {
		return nil, err
	}
	resp, err := decodeResponse(raw)
	if err != nil {
		return nil, err
	}
	if req.FailOnLanguageError {
		return resp, resp.Err()
	}
	return resp, nil
}

// Start starts a request and returns without waiting for its response.
// The execution must be closed with Execution.Close.
func (c *Client) Start(ctx context.Context, req Request) (*Execution, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	q := newEventQueue()
	h, err := c.b.start(ctx, c.envelope(req), q)
	if err != nil {
		return nil, err
	}
	return &Execution{handle: h, queue: q, failOnErr: req.FailOnLanguageError}, nil
}

// Check binds a request and classifies its effect without running it.
func (c *Client) Check(ctx context.Context, req Request) (*CheckResult, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	return c.b.check(ctx, c.envelope(req))
}

// Parse parses a program into a surface syntax tree nurt-ast/1. It has no
// effect on the world.
func (c *Client) Parse(ctx context.Context, program string, v Variant) (*ParseResult, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	return c.b.parse(ctx, program, variantOrDefault(v))
}

// Print prints a syntax tree nurt-ast/1 as canonical program text in the
// given notation.
func (c *Client) Print(ctx context.Context, ast json.RawMessage, v Variant) (string, error) {
	if err := c.usable(); err != nil {
		return "", err
	}
	return c.b.print(ctx, ast, variantOrDefault(v))
}

// Correct corrects the written form of a program without running it.
func (c *Client) Correct(ctx context.Context, program string, v Variant) (*CorrectResult, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	raw, err := c.b.correct(ctx, program, variantOrDefault(v))
	if err != nil {
		return nil, err
	}
	var res CorrectResult
	if err := json.Unmarshal(raw, &res); err != nil {
		return nil, newError(StatusInternal, "invalid correction result from nurt: "+err.Error(), err)
	}
	res.Raw = raw
	return &res, nil
}

// Page returns the next page of a paged result. A limit of 0 keeps the
// limit of the original request. An expired cursor is a language error in
// the response.
func (c *Client) Page(ctx context.Context, cursor string, limit int) (*Response, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	if limit < 0 {
		return nil, newError(StatusArg, "page limit must not be negative", nil)
	}
	raw, err := c.b.page(ctx, cursor, limit)
	if err != nil {
		return nil, err
	}
	return decodeResponse(raw)
}

// Describe returns the description of the language for tool integrations.
func (c *Client) Describe(ctx context.Context, q DescribeQuery) (string, error) {
	if err := c.usable(); err != nil {
		return "", err
	}
	if q.Language == "" {
		q.Language = c.opts.Language
	}
	return c.b.describe(ctx, q)
}

// Help returns the built-in help on a topic; an empty topic returns the
// index of topics.
func (c *Client) Help(ctx context.Context, topic string) (string, error) {
	if err := c.usable(); err != nil {
		return "", err
	}
	return c.b.help(ctx, topic)
}

// Catalog returns the command catalog as JSON. what is nil for the whole
// catalog or a JSON object selecting a group or a command.
func (c *Client) Catalog(ctx context.Context, what json.RawMessage) (json.RawMessage, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	return c.b.catalog(ctx, what)
}

// Schema returns a published JSON schema by name: "ast", "bound", "report",
// "request", "config", "catalog", "value" or "tool".
func (c *Client) Schema(ctx context.Context, which string) (json.RawMessage, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	return c.b.schema(ctx, which)
}

// History returns the journal history as JSON. query is nil or a JSON
// object with the parameters of the command history.
func (c *Client) History(ctx context.Context, query json.RawMessage) (json.RawMessage, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	return c.b.history(ctx, query)
}

// Space returns free space and journal usage as JSON for the volume that
// holds the given path, or for all volumes when volume is empty.
func (c *Client) Space(ctx context.Context, volume string) (json.RawMessage, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	return c.b.space(ctx, volume)
}

// Info returns information about the implementation as JSON: version,
// interface version, language and format versions, catalog revision,
// features, target and build.
func (c *Client) Info(ctx context.Context) (json.RawMessage, error) {
	if err := c.usable(); err != nil {
		return nil, err
	}
	return c.b.info(ctx)
}

// Close releases the client. With the process backend it asks nurt serve
// to finish running operations and waits for the process to exit. Close is
// idempotent.
func (c *Client) Close() error {
	if c.closed.Swap(true) {
		return nil
	}
	return c.b.close()
}

func variantOrDefault(v Variant) Variant {
	if v == "" {
		return VariantA
	}
	return v
}
