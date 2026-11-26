// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"bytes"
	"encoding/json"
)

// The library returns the results of nurt_parse and nurt_check as two
// documents: the syntax tree or the check document, which is null after a
// language error, and a document with the diagnostics and, after a language
// error, the error. The documents of nurt_correct, nurt_catalog, nurt_history
// and nurt_space are {"error": ...} after a language error. nurt serve answers
// every language error with the failure response nurt-response/1, whose
// report has the status failed, the name of the method as kind and the error.
// These functions build the answer of nurt serve from the documents.

// Builds the result of nurt_parse in the form of nurt serve.
func parseDocument(ast, diagnostics []byte) (json.RawMessage, error) {
	fields := map[string]json.RawMessage{"ast": json.RawMessage("null")}
	if len(bytes.TrimSpace(ast)) > 0 {
		fields["ast"] = ast
	}
	return withDiagnostics("parse", fields, diagnostics)
}

// Builds the result of nurt_check in the form of nurt serve.
func checkDocument(check, diagnostics []byte) (json.RawMessage, error) {
	fields := map[string]json.RawMessage{}
	if trimmed := bytes.TrimSpace(check); len(trimmed) > 0 && !bytes.Equal(trimmed, []byte("null")) {
		if err := json.Unmarshal(trimmed, &fields); err != nil {
			return nil, newError(StatusInternal, "invalid check document from libnurt: "+err.Error(), err)
		}
	}
	return withDiagnostics("check", fields, diagnostics)
}

func withDiagnostics(kind string, fields map[string]json.RawMessage, diagnostics []byte) (json.RawMessage, error) {
	var rest struct {
		Diagnostics json.RawMessage `json:"diagnostics"`
		Error       json.RawMessage `json:"error"`
	}
	if len(bytes.TrimSpace(diagnostics)) > 0 {
		if err := json.Unmarshal(diagnostics, &rest); err != nil {
			return nil, newError(StatusInternal, "invalid diagnostics from libnurt: "+err.Error(), err)
		}
	}
	if len(rest.Error) > 0 && string(bytes.TrimSpace(rest.Error)) != "null" {
		return failureResponse(kind, rest.Error)
	}
	if len(rest.Diagnostics) > 0 {
		fields["diagnostics"] = rest.Diagnostics
	}
	doc, err := json.Marshal(fields)
	if err != nil {
		return nil, newError(StatusInternal, "cannot encode the result of libnurt: "+err.Error(), err)
	}
	return doc, nil
}

// Builds the failure response of the language error of the method kind.
func failureResponse(kind string, languageError json.RawMessage) (json.RawMessage, error) {
	doc, err := json.Marshal(map[string]any{
		"nurt": "1.0",
		"report": map[string]any{
			"nurt":   "1.0",
			"status": "failed",
			"kind":   kind,
			"error":  languageError,
		},
	})
	if err != nil {
		return nil, newError(StatusInternal, "cannot encode the error of libnurt: "+err.Error(), err)
	}
	return doc, nil
}

// Returns the failure response of a document {"error": ...}, or the document
// itself when it is another one.
func queryDocument(kind string, doc []byte) (json.RawMessage, error) {
	var members map[string]json.RawMessage
	if json.Unmarshal(doc, &members) != nil || len(members) != 1 {
		return doc, nil
	}
	languageError, ok := members["error"]
	if !ok || string(bytes.TrimSpace(languageError)) == "null" {
		return doc, nil
	}
	return failureResponse(kind, languageError)
}
