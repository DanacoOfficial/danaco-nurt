// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"encoding/json"
	"strconv"
	"time"
)

// RequestFormat is the version of the request envelope nurt-request/1
// written by this package.
const RequestFormat = "1.0"

// Variant is a notation variant of the program text.
type Variant string

// Notation variants.
const (
	// VariantA is the word notation; it is used when the variant is empty.
	VariantA Variant = "a"
	// VariantB is the call notation.
	VariantB Variant = "b"
	// VariantC is the named notation.
	VariantC Variant = "c"
)

// Format selects the rendering of the response text.
type Format string

// Response formats.
const (
	FormatText     Format = "text"
	FormatJSON     Format = "json"
	FormatMarkdown Format = "markdown"
	FormatCompact  Format = "compact"
)

// Verbosity selects the amount of detail in the response.
type Verbosity string

// Verbosity levels.
const (
	VerbosityQuiet    Verbosity = "quiet"
	VerbosityNormal   Verbosity = "normal"
	VerbosityDetailed Verbosity = "detailed"
)

// DiffMode selects how changes are shown in the response.
type DiffMode string

// Diff modes.
const (
	DiffNone    DiffMode = "none"
	DiffStat    DiffMode = "stat"
	DiffUnified DiffMode = "unified"
)

// Request is a request envelope nurt-request/1. Exactly one of Program, AST
// and Cursor is normally set.
type Request struct {
	// Program is the program text.
	Program string `json:"program,omitempty"`
	// Variant is the notation of Program; empty means the word notation
	// or the configured default.
	Variant Variant `json:"variant,omitempty"`
	// AST is a surface syntax tree nurt-ast/1, equivalent to Program.
	AST json.RawMessage `json:"ast,omitempty"`
	// Cursor continues an earlier paged result.
	Cursor string `json:"cursor,omitempty"`
	// Accept selects a candidate of an earlier E_AMBIGUOUS error.
	Accept string `json:"accept,omitempty"`
	// Options are the call options.
	Options Options `json:"options"`

	// FailOnLanguageError makes Run and Execution.Wait return the language
	// error of the response as a *LanguageError, together with the response.
	FailOnLanguageError bool `json:"-"`
}

// Options are the call options shared by every command. A zero field takes
// its value from the configuration or the built-in default.
type Options struct {
	// Preview computes targets and effect without changing anything.
	Preview bool `json:"preview,omitempty"`
	// Base is the absolute directory against which relative paths are
	// resolved.
	Base string `json:"base,omitempty"`
	// Limit is the number of records or changes per page (1 to 5000).
	Limit int `json:"limit,omitempty"`
	// Fields lists the fields of the result to keep.
	Fields []string `json:"fields,omitempty"`
	// Format selects the rendering of the response text.
	Format Format `json:"format,omitempty"`
	// Verbosity selects the amount of detail.
	Verbosity Verbosity `json:"verbosity,omitempty"`
	// Diff selects how changes are shown.
	Diff DiffMode `json:"diff,omitempty"`
	// DiffContext is the number of context lines of a unified diff; nil
	// keeps the default.
	DiffContext *int `json:"diff_context,omitempty"`
	// MaxChars is the largest number of characters of a single text field;
	// 0 keeps the default.
	MaxChars int `json:"max_chars,omitempty"`
	// SaveTo is the absolute path of a file that receives the full result.
	SaveTo string `json:"save_to,omitempty"`
	// Timeout is the time limit of the whole program, with millisecond
	// resolution; 0 keeps the default.
	Timeout time.Duration `json:"-"`
	// Session is the session identifier recorded in the journal; empty
	// uses the session of the client.
	Session string `json:"session,omitempty"`
	// Language is the language of messages and help ("en" or "pl"); empty
	// uses the language of the client.
	Language string `json:"language,omitempty"`
	// Script runs several pipelines, each as its own operation.
	Script bool `json:"script,omitempty"`
}

// MarshalJSON encodes the options, writing Timeout in the duration syntax
// of the language, for example "1500ms" or "90s".
func (o Options) MarshalJSON() ([]byte, error) {
	type plain Options
	return json.Marshal(struct {
		plain
		Timeout string `json:"timeout,omitempty"`
	}{plain: plain(o), Timeout: formatTimeout(o.Timeout)})
}

// Durations are rounded up to whole milliseconds, because the language has
// no smaller unit.
func formatTimeout(d time.Duration) string {
	if d <= 0 {
		return ""
	}
	if d%time.Second == 0 {
		return strconv.FormatInt(int64(d/time.Second), 10) + "s"
	}
	ms := (d + time.Millisecond - 1) / time.Millisecond
	return strconv.FormatInt(int64(ms), 10) + "ms"
}

type envelope struct {
	Nurt string `json:"nurt"`
	Request
}

func (c *Client) envelope(req Request) envelope {
	if req.Options.Session == "" {
		req.Options.Session = c.opts.Session
	}
	if req.Options.Language == "" {
		req.Options.Language = c.opts.Language
	}
	return envelope{Nurt: RequestFormat, Request: req}
}

// DescribeQuery selects the description returned by Client.Describe.
type DescribeQuery struct {
	// Layer is "core" (the default), "group" or another layer of the
	// description.
	Layer string `json:"layer,omitempty"`
	// Name selects a group or command within the layer.
	Name string `json:"name,omitempty"`
	// Language is "en" or "pl"; empty uses the language of the client.
	Language string `json:"language,omitempty"`
}
