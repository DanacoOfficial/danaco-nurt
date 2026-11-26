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
// error, the error. nurt serve returns one document with the members ast or
// bound, diagnostics and error. These functions build that one document.

// Builds the result of nurt_parse in the form of nurt serve.
func parseDocument(ast, diagnostics []byte) (json.RawMessage, error) {
	fields := map[string]json.RawMessage{"ast": json.RawMessage("null")}
	if len(bytes.TrimSpace(ast)) > 0 {
		fields["ast"] = ast
	}
	return withDiagnostics(fields, diagnostics)
}

// Builds the result of nurt_check in the form of nurt serve.
func checkDocument(check, diagnostics []byte) (json.RawMessage, error) {
	fields := map[string]json.RawMessage{}
	if trimmed := bytes.TrimSpace(check); len(trimmed) > 0 && !bytes.Equal(trimmed, []byte("null")) {
		if err := json.Unmarshal(trimmed, &fields); err != nil {
			return nil, newError(StatusInternal, "invalid check document from libnurt: "+err.Error(), err)
		}
	}
	return withDiagnostics(fields, diagnostics)
}

func withDiagnostics(fields map[string]json.RawMessage, diagnostics []byte) (json.RawMessage, error) {
	var rest struct {
		Diagnostics json.RawMessage `json:"diagnostics"`
		Error       json.RawMessage `json:"error"`
	}
	if len(bytes.TrimSpace(diagnostics)) > 0 {
		if err := json.Unmarshal(diagnostics, &rest); err != nil {
			return nil, newError(StatusInternal, "invalid diagnostics from libnurt: "+err.Error(), err)
		}
	}
	if len(rest.Diagnostics) > 0 {
		fields["diagnostics"] = rest.Diagnostics
	}
	if len(rest.Error) > 0 && string(bytes.TrimSpace(rest.Error)) != "null" {
		fields["error"] = rest.Error
	}
	doc, err := json.Marshal(fields)
	if err != nil {
		return nil, newError(StatusInternal, "cannot encode the result of libnurt: "+err.Error(), err)
	}
	return doc, nil
}
