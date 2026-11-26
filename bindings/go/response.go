// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"bytes"
	"encoding/json"
)

// ReportStatus is the outcome recorded in an effect report.
type ReportStatus string

// Report statuses.
const (
	ReportOK         ReportStatus = "ok"
	ReportPlanned    ReportStatus = "planned"
	ReportRunning    ReportStatus = "running"
	ReportPartial    ReportStatus = "partial"
	ReportRolledBack ReportStatus = "rolled_back"
	ReportFailed     ReportStatus = "failed"
)

// Report holds the fields of the effect report nurt-report/1 that every
// caller needs. The complete report is available as JSON in Raw.
type Report struct {
	// Nurt is the format version of the report.
	Nurt string `json:"nurt"`
	// Status is the outcome of the operation.
	Status ReportStatus `json:"status"`
	// Kind is the kind of operation, for example "replace" or "find".
	Kind string `json:"kind"`
	// Op is the number of the journal entry, or 0 when there is none.
	Op int64 `json:"op,omitempty"`
	// Ran is the canonical text of the program that ran, present when
	// corrections were applied.
	Ran string `json:"ran,omitempty"`
	// Error is the language error, or nil.
	Error *LanguageError `json:"error,omitempty"`
	// NextCursor is the cursor of the next page; empty on the last page.
	NextCursor string `json:"next_cursor,omitempty"`

	// Raw is the complete report as JSON.
	Raw json.RawMessage `json:"-"`
}

// UnmarshalJSON decodes the report and keeps a copy in Raw.
func (r *Report) UnmarshalJSON(data []byte) error {
	type plain Report
	var p plain
	if err := json.Unmarshal(data, &p); err != nil {
		return err
	}
	*r = Report(p)
	r.Raw = append(json.RawMessage(nil), data...)
	return nil
}

// Rendered is the response text in the requested format.
type Rendered struct {
	Format Format `json:"format"`
	Text   string `json:"text"`
}

// Response is a response nurt-response/1: the effect report and, for text
// formats, the rendered text. A language error is part of the response, not
// a failure of the call.
type Response struct {
	// Nurt is the format version of the response.
	Nurt string `json:"nurt"`
	// Report is the effect report.
	Report Report `json:"report"`
	// Rendered is the rendered text, or nil for the JSON format.
	Rendered *Rendered `json:"rendered,omitempty"`

	// Raw is the complete response as JSON.
	Raw json.RawMessage `json:"-"`
}

// OK reports whether the response carries no language error.
func (r *Response) OK() bool { return r.Report.Error == nil }

// Err returns the language error of the response, or nil.
func (r *Response) Err() error {
	if r.Report.Error == nil {
		return nil
	}
	return r.Report.Error
}

// Text returns the rendered text, or an empty string when the response has
// none.
func (r *Response) Text() string {
	if r.Rendered == nil {
		return ""
	}
	return r.Rendered.Text
}

// NextCursor returns the cursor of the next page, or an empty string on the
// last page.
func (r *Response) NextCursor() string { return r.Report.NextCursor }

func decodeResponse(raw json.RawMessage) (*Response, error) {
	var resp Response
	if err := json.Unmarshal(raw, &resp); err != nil {
		return nil, newError(StatusInternal, "invalid response from nurt: "+err.Error(), err)
	}
	resp.Raw = raw
	return &resp, nil
}

// CheckResult is the result of Client.Check: the bound tree with the
// classified effect of the request, and diagnostics.
type CheckResult struct {
	// Bound is the bound tree nurt-bound/1 with targets, paths, external
	// programs, network addresses, corrections and notable effects; empty
	// when the request does not bind.
	Bound json.RawMessage `json:"bound"`
	// Diagnostics lists positions and messages.
	Diagnostics json.RawMessage `json:"diagnostics,omitempty"`
	// Error is the language error that stopped the binding, or nil.
	Error *LanguageError `json:"error,omitempty"`

	// Raw is the complete check document as JSON.
	Raw json.RawMessage `json:"-"`
}

// UnmarshalJSON decodes the check document and keeps a copy in Raw.
func (r *CheckResult) UnmarshalJSON(data []byte) error {
	type plain CheckResult
	var p plain
	if err := json.Unmarshal(data, &p); err != nil {
		return err
	}
	*r = CheckResult(p)
	if r.Error == nil {
		r.Error = reportError(data)
	}
	r.Bound = emptyIfNull(r.Bound)
	r.Diagnostics = emptyIfNull(r.Diagnostics)
	r.Raw = append(json.RawMessage(nil), data...)
	return nil
}

// ParseResult is the result of Client.Parse.
type ParseResult struct {
	// AST is the surface syntax tree nurt-ast/1; empty when the program
	// does not parse.
	AST json.RawMessage `json:"ast"`
	// Diagnostics lists positions and messages.
	Diagnostics json.RawMessage `json:"diagnostics,omitempty"`
	// Error is the language error that stopped the parse, or nil.
	Error *LanguageError `json:"error,omitempty"`

	// Raw is the complete result as JSON.
	Raw json.RawMessage `json:"-"`
}

// UnmarshalJSON decodes the result and keeps a copy in Raw.
func (r *ParseResult) UnmarshalJSON(data []byte) error {
	type plain ParseResult
	var p plain
	if err := json.Unmarshal(data, &p); err != nil {
		return err
	}
	*r = ParseResult(p)
	if r.Error == nil {
		r.Error = reportError(data)
	}
	r.AST = emptyIfNull(r.AST)
	r.Diagnostics = emptyIfNull(r.Diagnostics)
	r.Raw = append(json.RawMessage(nil), data...)
	return nil
}

// Returns the language error that the server reports in the report of a
// failure response nurt-response/1, or nil.
func reportError(data []byte) *LanguageError {
	var envelope struct {
		Report struct {
			Error *LanguageError `json:"error"`
		} `json:"report"`
	}
	if json.Unmarshal(data, &envelope) != nil {
		return nil
	}
	return envelope.Report.Error
}

// A member that is JSON null is an absent value.
func emptyIfNull(raw json.RawMessage) json.RawMessage {
	if string(bytes.TrimSpace(raw)) == "null" {
		return nil
	}
	return raw
}

// CorrectResult is the result of Client.Correct.
type CorrectResult struct {
	// Program is the corrected program text.
	Program string `json:"program"`
	// Corrections lists the corrections that were applied.
	Corrections json.RawMessage `json:"corrections,omitempty"`

	// Raw is the complete result as JSON.
	Raw json.RawMessage `json:"-"`
}
