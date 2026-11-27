// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/go/errors.go
// Responsibility:   Source file bindings/go/errors.go.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package nurt

import (
	"context"
	"encoding/json"
	"errors"
	"strconv"
	"strings"
)

// Status is the outcome of a call, with the values of nurt_status in the C
// interface. A later minor version of the interface may add values; any
// unknown non-zero value is a failure.
type Status int

// Status values. They are stable within a major version of the interface.
const (
	StatusOK        Status = 0
	StatusTimeout   Status = 1
	StatusCancelled Status = 2
	StatusArg       Status = 3
	StatusABI       Status = 4
	StatusIO        Status = 5
	StatusInternal  Status = 6
)

var statusNames = [...]string{
	StatusOK:        "NURT_OK",
	StatusTimeout:   "NURT_E_TIMEOUT",
	StatusCancelled: "NURT_E_CANCELLED",
	StatusArg:       "NURT_E_ARG",
	StatusABI:       "NURT_E_ABI",
	StatusIO:        "NURT_E_IO",
	StatusInternal:  "NURT_E_INTERNAL",
}

// String returns the name of the status in the C interface, for example
// "NURT_E_ARG".
func (s Status) String() string {
	if s >= 0 && int(s) < len(statusNames) {
		return statusNames[s]
	}
	return "NURT_E_UNKNOWN(" + strconv.Itoa(int(s)) + ")"
}

// Error is a failure of a call: an invalid argument, a missing or
// incompatible nurt executable or library, a failed process, a cancelled
// or timed-out call, or an internal failure. Errors of the language are not
// reported as Error; they are part of the response (see Response.Err).
type Error struct {
	// Status classifies the failure.
	Status Status
	// Message describes the failure.
	Message string
	// Detail holds additional JSON data supplied by nurt, if any.
	Detail json.RawMessage
	// Checked lists the locations searched for the nurt executable when it
	// was not found.
	Checked []string

	cause error
}

func (e *Error) Error() string {
	var b strings.Builder
	b.WriteString("nurt: ")
	b.WriteString(e.Status.String())
	b.WriteString(": ")
	b.WriteString(e.Message)
	if len(e.Checked) > 0 {
		b.WriteString(" (checked: ")
		b.WriteString(strings.Join(e.Checked, ", "))
		b.WriteString(")")
	}
	return b.String()
}

// Unwrap returns the underlying cause, such as context.Canceled or
// context.DeadlineExceeded for an interrupted call.
func (e *Error) Unwrap() error { return e.cause }

func newError(s Status, msg string, cause error) *Error {
	return &Error{Status: s, Message: msg, cause: cause}
}

// A context deadline becomes StatusTimeout and a cancellation
// StatusCancelled; both unwrap to the context error.
func contextError(err error) *Error {
	if errors.Is(err, context.DeadlineExceeded) {
		return newError(StatusTimeout, "call exceeded the context deadline", err)
	}
	return newError(StatusCancelled, "call was cancelled by the context", err)
}

// LanguageError is an error of the Danaco Nurt language: a syntax, binding,
// assertion, stale target, ambiguity or journal error reported in the effect
// report with a stable E_* code. It is returned as an error when the
// request sets FailOnLanguageError, and by the queries that have no response
// to carry it (Describe, Help and Print) when the process backend reports one;
// an unknown value of an argument of such a query is an *Error with
// StatusArg.
type LanguageError struct {
	// Code is the stable error code, for example "E_SYNTAX".
	Code string `json:"code"`
	// Message describes the error.
	Message string `json:"message"`
	// At is the position of the error: the execution stage and the place in
	// the program text.
	At *Position `json:"at,omitempty"`
	// Fix lists corrected forms of the program.
	Fix []string `json:"fix,omitempty"`
	// Help names the help topic that explains the error.
	Help string `json:"help,omitempty"`

	// Raw is the complete error object as JSON.
	Raw json.RawMessage `json:"-"`
}

// Position is the position of a language error: a location in program text,
// whose lines and columns start at 1, and the execution stage that reported
// the error.
type Position struct {
	Line   int `json:"line"`
	Column int `json:"column"`
	Length int `json:"length,omitempty"`
	// Stage is the execution stage in which the error occurred, for example
	// "0" or "5a".
	Stage string `json:"stage,omitempty"`
}

func (e *LanguageError) Error() string {
	return "nurt: " + e.Code + ": " + e.Message
}

// UnmarshalJSON decodes the error object and keeps a copy in Raw.
func (e *LanguageError) UnmarshalJSON(data []byte) error {
	type plain LanguageError
	var p plain
	if err := json.Unmarshal(data, &p); err != nil {
		return err
	}
	*e = LanguageError(p)
	e.Raw = append(json.RawMessage(nil), data...)
	return nil
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
