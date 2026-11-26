// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"encoding/json"
	"testing"
)

func TestEventTimeIsTextOrNumber(t *testing.T) {
	cases := []struct{ raw, want string }{
		{`{"t":"exec.started","seq":1,"time":"2026-01-01T00:00:00Z"}`, "2026-01-01T00:00:00Z"},
		{`{"t":"exec.started","seq":1,"time":1795392000000000000}`, "1795392000000000000"},
		{`{"t":"exec.started","seq":1}`, ""},
		{`{"t":"exec.started","seq":1,"time":null}`, ""},
	}
	for _, c := range cases {
		ev, ok := decodeEvent([]byte(c.raw))
		if !ok {
			t.Errorf("event %s was rejected", c.raw)
			continue
		}
		if ev.Time != c.want || ev.Type != "exec.started" || ev.Seq != 1 || string(ev.Raw) != c.raw {
			t.Errorf("event %s decoded as %+v", c.raw, ev)
		}
	}
	if _, ok := decodeEvent([]byte(`{"t":`)); ok {
		t.Error("truncated event was accepted")
	}
}

func TestEventJobIdentifierIsIDOrJobID(t *testing.T) {
	cases := []struct{ raw, want string }{
		{`{"t":"job.started","seq":2,"id":"j1"}`, "j1"},
		{`{"t":"job.output","seq":3,"job_id":"j2"}`, "j2"},
		{`{"t":"job.output","seq":3,"id":"j3","job_id":"j4"}`, "j4"},
		{`{"t":"progress","seq":4,"id":7}`, "7"},
		{`{"t":"exec.finished","seq":5,"status":"ok","id":null}`, ""},
	}
	for _, c := range cases {
		ev, ok := decodeEvent([]byte(c.raw))
		if !ok || ev.JobID != c.want {
			t.Errorf("event %s: job %q, ok %v, want %q", c.raw, ev.JobID, ok, c.want)
		}
	}
}

func TestParseDocumentKeepsTheLanguageError(t *testing.T) {
	diagnostics := `{"diagnostics":[{"message":"m"}],"error":{"code":"E_LEXICAL","message":"unterminated text"}}`
	doc, err := parseDocument([]byte("null"), []byte(diagnostics))
	if err != nil {
		t.Fatal(err)
	}
	var res ParseResult
	if err := json.Unmarshal(doc, &res); err != nil {
		t.Fatal(err)
	}
	if len(res.AST) != 0 {
		t.Errorf("AST = %s, want none", res.AST)
	}
	if res.Error == nil || res.Error.Code != "E_LEXICAL" {
		t.Errorf("Error = %+v", res.Error)
	}
	if string(res.Diagnostics) != `[{"message":"m"}]` {
		t.Errorf("Diagnostics = %s", res.Diagnostics)
	}
	if len(res.Raw) == 0 {
		t.Error("Raw is empty")
	}
}

func TestParseDocumentOfATree(t *testing.T) {
	doc, err := parseDocument([]byte(`{"nurt":"1.0"}`), []byte(`{"diagnostics":[]}`))
	if err != nil {
		t.Fatal(err)
	}
	var res ParseResult
	if err := json.Unmarshal(doc, &res); err != nil {
		t.Fatal(err)
	}
	if string(res.AST) != `{"nurt":"1.0"}` || res.Error != nil || string(res.Diagnostics) != "[]" {
		t.Errorf("result = %+v", res)
	}
	if _, err := parseDocument([]byte("{}"), []byte("[")); err == nil {
		t.Error("invalid diagnostics were accepted")
	}
}

func TestCheckDocumentTakesTheBoundMemberOfTheCheckDocument(t *testing.T) {
	check := `{"bound":{"nurt":"1.0","program":"x"},"ast_hash":"sha256:ab","effect":"read"}`
	doc, err := checkDocument([]byte(check), []byte(`{"diagnostics":[]}`))
	if err != nil {
		t.Fatal(err)
	}
	var res CheckResult
	if err := json.Unmarshal(doc, &res); err != nil {
		t.Fatal(err)
	}
	if string(res.Bound) != `{"nurt":"1.0","program":"x"}` {
		t.Errorf("Bound = %s", res.Bound)
	}
	var raw map[string]json.RawMessage
	if err := json.Unmarshal(res.Raw, &raw); err != nil || string(raw["effect"]) != `"read"` {
		t.Errorf("Raw = %s, %v", res.Raw, err)
	}
}

func TestCheckDocumentAfterALanguageError(t *testing.T) {
	diagnostics := `{"diagnostics":[],"error":{"code":"E_UNKNOWN_COMMAND","message":"no such command"}}`
	doc, err := checkDocument([]byte("null"), []byte(diagnostics))
	if err != nil {
		t.Fatal(err)
	}
	var res CheckResult
	if err := json.Unmarshal(doc, &res); err != nil {
		t.Fatal(err)
	}
	if len(res.Bound) != 0 || res.Error == nil || res.Error.Code != "E_UNKNOWN_COMMAND" {
		t.Errorf("result = %+v", res)
	}
	if _, err := checkDocument([]byte("[1]"), nil); err == nil {
		t.Error("a check document that is not an object was accepted")
	}
}

func TestNullMembersOfResultsAreAbsent(t *testing.T) {
	var parsed ParseResult
	if err := json.Unmarshal([]byte(`{"ast":null,"diagnostics":null}`), &parsed); err != nil {
		t.Fatal(err)
	}
	if len(parsed.AST) != 0 || len(parsed.Diagnostics) != 0 {
		t.Errorf("parse result = %+v", parsed)
	}
	var checked CheckResult
	if err := json.Unmarshal([]byte(`{"bound":null}`), &checked); err != nil {
		t.Fatal(err)
	}
	if len(checked.Bound) != 0 {
		t.Errorf("check result = %+v", checked)
	}
}
