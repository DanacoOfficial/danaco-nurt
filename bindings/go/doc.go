// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

// Package nurt is the Go binding of Danaco Nurt, a command shell and language
// where every change can be undone.
//
// The binding is thin: requests, reports, syntax trees and events are JSON
// documents defined by the published schemas, and the binding passes them
// between Go and an implementation of Danaco Nurt.
//
// # Backends
//
// BackendProcess, the default, starts nurt serve --stdio as a child process
// and talks to it with JSON-RPC 2.0 (protocol nurt-rpc/1). It needs only the
// Go standard library, so the module builds without a C compiler and for
// any target. A process that ends unexpectedly fails the calls in flight
// and is started again on the next call.
//
// BackendLibrary calls libnurt in process through cgo. It is compiled only
// with the nurt_cgo build tag and links libnurt found by pkg-config:
//
//	go build -tags nurt_cgo
//
// # Locating nurt
//
// The process backend runs ClientOptions.Executable when it is set,
// otherwise the file named by the NURT_EXECUTABLE environment variable,
// otherwise nurt found in PATH. When none is found, New returns an *Error
// with StatusIO that lists the locations checked.
//
// # Errors
//
// Errors of the language, such as a syntax error or an ambiguous target,
// are part of the response: Response.OK reports false and Response.Err
// returns a *LanguageError with a stable E_* code. Setting
// Request.FailOnLanguageError also returns that error from Run.
//
// Failures of the call itself, such as an invalid argument, a missing
// executable, an incompatible version, a failed process or an ended
// context, are returned as *Error with a Status named after nurt_status of
// the C interface.
//
// # Concurrency
//
// A Client is safe for concurrent use; requests run in parallel and every
// call takes a context.Context. When the context ends, the call returns at
// once and the request is cancelled in nurt.
package nurt
