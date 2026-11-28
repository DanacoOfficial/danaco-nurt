// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/go/conformance_test.go
// Responsibility:   Source file bindings/go/conformance_test.go.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

package nurt

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"maps"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"
)

// The runner executes the language-neutral binding cases of the conformance
// suite. Each case file holds one JSON object: a call of the API and the
// expected outcome, or a list of steps whose saved values are substituted
// into later arguments as ${name}.
const (
	conformanceDirEnv = "NURT_CONFORMANCE_BINDINGS"
	testCommandEnv    = "NURT_TEST_COMMAND"
	caseTimeout       = 60 * time.Second
	optionalPrefix    = "optional:"
)

var (
	defaultConformanceDir = filepath.Join("..", "..", "conformance", "bindings")
	fakeConformanceDir    = filepath.Join("testdata", "conformance")
)

type bindingCase struct {
	ID       string     `json:"id"`
	Class    string     `json:"class"`
	Backends []Backend  `json:"backends"`
	Call     *caseCall  `json:"call"`
	Expect   caseExpect `json:"expect"`
	Steps    []caseStep `json:"steps"`
}

type caseStep struct {
	Call   caseCall          `json:"call"`
	Expect caseExpect        `json:"expect"`
	Save   map[string]string `json:"save"`
}

type caseCall struct {
	Method string          `json:"method"`
	Args   json.RawMessage `json:"args"`
}

type caseExpect struct {
	OK              *bool                      `json:"ok"`
	ErrorCode       string                     `json:"error_code"`
	InvocationError string                     `json:"invocation_error"`
	Fields          map[string]json.RawMessage `json:"fields"`
	Events          []string                   `json:"events"`
	TextContains    string                     `json:"text_contains"`
}

func (tc bindingCase) steps() []caseStep {
	if tc.Call != nil {
		return []caseStep{{Call: *tc.Call, Expect: tc.Expect}}
	}
	return tc.Steps
}

func loadCases(dir string) ([]bindingCase, error) {
	paths, err := filepath.Glob(filepath.Join(dir, "*.json"))
	if err != nil {
		return nil, err
	}
	if _, err := os.Stat(dir); err != nil {
		return nil, err
	}
	slices.Sort(paths)
	cases := make([]bindingCase, 0, len(paths))
	for _, p := range paths {
		raw, err := os.ReadFile(p)
		if err != nil {
			return nil, err
		}
		var tc bindingCase
		if err := json.Unmarshal(raw, &tc); err != nil {
			return nil, fmt.Errorf("%s: %w", p, err)
		}
		if tc.ID == "" || (tc.Call == nil) == (len(tc.Steps) == 0) {
			return nil, fmt.Errorf("%s: a case needs an id and either call or steps", p)
		}
		cases = append(cases, tc)
	}
	return cases, nil
}

type outcome struct {
	obs    any
	text   string
	events []string
	err    error
}

func decodeObservation(raw []byte) any {
	var v any
	if json.Unmarshal(raw, &v) != nil {
		return nil
	}
	return v
}

func responseOutcome(resp *Response, err error) outcome {
	out := outcome{err: err}
	if resp != nil {
		out.obs = decodeObservation(resp.Raw)
		out.text = resp.Text()
	}
	return out
}

func textOutcome(text string, err error) outcome {
	return outcome{obs: map[string]any{"text": text}, text: text, err: err}
}

func jsonOutcome(raw json.RawMessage, err error) outcome {
	return outcome{obs: decodeObservation(raw), err: err}
}

func perform(ctx context.Context, c *Client, call caseCall) (outcome, error) {
	args := call.Args
	if len(args) == 0 {
		args = json.RawMessage("{}")
	}
	var a struct {
		Request
		Cancel              bool            `json:"cancel"`
		FailOnLanguageError bool            `json:"fail_on_language_error"`
		Limit               int             `json:"limit"`
		Layer               string          `json:"layer"`
		Name                string          `json:"name"`
		Language            string          `json:"language"`
		Topic               string          `json:"topic"`
		What                json.RawMessage `json:"what"`
		Which               string          `json:"which"`
		Query               json.RawMessage `json:"query"`
		Volume              string          `json:"volume"`
	}
	if err := json.Unmarshal(args, &a); err != nil {
		return outcome{}, fmt.Errorf("arguments of %s: %w", call.Method, err)
	}
	req := a.Request
	req.FailOnLanguageError = a.FailOnLanguageError
	switch call.Method {
	case "run":
		return responseOutcome(c.Run(ctx, req)), nil
	case "start":
		return startOutcome(ctx, c, req, a.Cancel), nil
	case "check":
		res, err := c.Check(ctx, req)
		if err != nil {
			return outcome{err: err}, nil
		}
		return outcome{obs: map[string]any{"bound": decodeObservation(res.Bound), "diagnostics": decodeObservation(res.Diagnostics)}}, nil
	case "parse":
		res, err := c.Parse(ctx, req.Program, req.Variant)
		if err != nil {
			return outcome{err: err}, nil
		}
		return outcome{obs: map[string]any{"ast": decodeObservation(res.AST), "diagnostics": decodeObservation(res.Diagnostics)}}, nil
	case "print":
		return textOutcome(c.Print(ctx, req.AST, req.Variant)), nil
	case "correct":
		res, err := c.Correct(ctx, req.Program, req.Variant)
		if err != nil {
			return outcome{err: err}, nil
		}
		return outcome{obs: decodeObservation(res.Raw), text: res.Program}, nil
	case "page":
		return responseOutcome(c.Page(ctx, req.Cursor, a.Limit)), nil
	case "describe":
		return textOutcome(c.Describe(ctx, DescribeQuery{Layer: a.Layer, Name: a.Name, Language: a.Language})), nil
	case "help":
		return textOutcome(c.Help(ctx, a.Topic)), nil
	case "catalog":
		return jsonOutcome(c.Catalog(ctx, a.What)), nil
	case "schema":
		return jsonOutcome(c.Schema(ctx, a.Which)), nil
	case "history":
		return jsonOutcome(c.History(ctx, a.Query)), nil
	case "space":
		return jsonOutcome(c.Space(ctx, a.Volume)), nil
	case "info":
		return jsonOutcome(c.Info(ctx)), nil
	}
	return outcome{}, fmt.Errorf("unknown method %q", call.Method)
}

func startOutcome(ctx context.Context, c *Client, req Request, cancel bool) outcome {
	x, err := c.Start(ctx, req)
	if err != nil {
		return outcome{err: err}
	}
	defer x.Close()
	if cancel {
		if err := x.Cancel(ctx); err != nil {
			return outcome{err: err}
		}
	}
	var events []string
	for ev := range x.Events() {
		events = append(events, ev.Type)
	}
	out := responseOutcome(x.Wait(ctx))
	out.events = events
	return out
}

func lookup(v any, path string) (any, bool) {
	for _, part := range strings.Split(path, ".") {
		switch node := v.(type) {
		case map[string]any:
			next, ok := node[part]
			if !ok {
				return nil, false
			}
			v = next
		case []any:
			i, err := strconv.Atoi(part)
			if err != nil || i < 0 || i >= len(node) {
				return nil, false
			}
			v = node[i]
		default:
			return nil, false
		}
	}
	return v, true
}

func isSubsequence(want, got []string) bool {
	i := 0
	for _, g := range got {
		if i < len(want) && g == want[i] {
			i++
		}
	}
	return i == len(want)
}

func verify(exp caseExpect, out outcome) []string {
	var problems []string
	fail := func(format string, args ...any) { problems = append(problems, fmt.Sprintf(format, args...)) }

	var callErr *Error
	isCallErr := errors.As(out.err, &callErr)
	switch {
	case exp.InvocationError != "" && !isCallErr:
		fail("want invocation error %s, got %v", exp.InvocationError, out.err)
	case exp.InvocationError != "" && callErr.Status.String() != exp.InvocationError:
		fail("want invocation error %s, got %s: %s", exp.InvocationError, callErr.Status, callErr.Message)
	case exp.InvocationError == "" && isCallErr:
		fail("unexpected invocation error: %v", out.err)
	}

	if exp.OK != nil {
		_, hasError := lookup(out.obs, "report.error")
		if _, hasReport := lookup(out.obs, "report"); !hasReport {
			fail("want ok %v, got no response", *exp.OK)
		} else if hasError == *exp.OK {
			fail("want ok %v, got report %v", *exp.OK, out.obs)
		}
	}
	if exp.ErrorCode != "" {
		if code, _ := lookup(out.obs, "report.error.code"); code != exp.ErrorCode {
			fail("want error code %s, got %v", exp.ErrorCode, code)
		}
	}
	for _, path := range slices.Sorted(maps.Keys(exp.Fields)) {
		var want any
		if err := json.Unmarshal(exp.Fields[path], &want); err != nil {
			fail("expected value of %s is not JSON: %v", path, err)
			continue
		}
		got, ok := lookup(out.obs, path)
		if !ok {
			fail("field %s is missing", path)
			continue
		}
		if !reflect.DeepEqual(got, want) {
			fail("field %s = %v, want %v", path, got, want)
		}
	}
	if len(exp.Events) > 0 && !isSubsequence(exp.Events, out.events) {
		fail("events %v do not contain %v in order", out.events, exp.Events)
	}
	if exp.TextContains != "" && !strings.Contains(out.text, exp.TextContains) {
		fail("text %q does not contain %q", out.text, exp.TextContains)
	}
	return problems
}

func substitute(args json.RawMessage, vars map[string]string) json.RawMessage {
	if len(vars) == 0 {
		return args
	}
	s := string(args)
	for name, value := range vars {
		quoted, _ := json.Marshal(value)
		s = strings.ReplaceAll(s, "${"+name+"}", string(quoted[1:len(quoted)-1]))
	}
	return json.RawMessage(s)
}

func runCase(ctx context.Context, c *Client, tc bindingCase) []string {
	var problems []string
	vars := map[string]string{}
	for i, step := range tc.steps() {
		call := step.Call
		call.Args = substitute(call.Args, vars)
		out, err := perform(ctx, c, call)
		if err != nil {
			return append(problems, fmt.Sprintf("step %d: %v", i+1, err))
		}
		for _, p := range verify(step.Expect, out) {
			problems = append(problems, fmt.Sprintf("step %d (%s): %s", i+1, call.Method, p))
		}
		for name, path := range step.Save {
			v, ok := lookup(out.obs, path)
			s, isString := v.(string)
			if !ok || !isString {
				problems = append(problems, fmt.Sprintf("step %d: cannot save %s from %s", i+1, name, path))
				continue
			}
			vars[name] = s
		}
	}
	return problems
}

func hasFeature(ctx context.Context, c *Client, feature string) bool {
	raw, err := c.Info(ctx)
	if err != nil {
		return false
	}
	var info struct {
		Features []string `json:"features"`
	}
	return json.Unmarshal(raw, &info) == nil && slices.Contains(info.Features, feature)
}

func runConformance(t *testing.T, cases []bindingCase, backends []Backend, newClient func(*testing.T, Backend) *Client) {
	t.Helper()
	for _, b := range backends {
		for _, tc := range cases {
			if len(tc.Backends) > 0 && !slices.Contains(tc.Backends, b) {
				continue
			}
			t.Run(string(b)+"/"+tc.ID, func(t *testing.T) {
				ctx, cancel := context.WithTimeout(context.Background(), caseTimeout)
				defer cancel()
				c := newClient(t, b)
				if feature, ok := strings.CutPrefix(tc.Class, optionalPrefix); ok && !hasFeature(ctx, c, feature) {
					t.Skipf("implementation does not declare feature %s", feature)
				}
				for _, p := range runCase(ctx, c, tc) {
					t.Error(p)
				}
			})
		}
	}
}

func TestConformanceRunnerOnFakeServer(t *testing.T) {
	cases, err := loadCases(fakeConformanceDir)
	if err != nil {
		t.Fatal(err)
	}
	if len(cases) == 0 {
		t.Fatal("no cases in testdata")
	}
	runConformance(t, cases, []Backend{BackendProcess}, func(t *testing.T, _ Backend) *Client {
		return newTestClient(t, fakeOptions(t, ""))
	})
}

func TestConformanceRunnerDetectsMismatch(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	ctx := context.Background()
	no := false
	call := &caseCall{Method: "run", Args: json.RawMessage(`{"program":"info"}`)}
	cases := map[string]caseExpect{
		"ok":               {OK: &no},
		"error code":       {ErrorCode: "E_SYNTAX"},
		"invocation error": {InvocationError: "NURT_E_ARG"},
		"field value":      {Fields: map[string]json.RawMessage{"report.kind": json.RawMessage(`"find"`)}},
		"missing field":    {Fields: map[string]json.RawMessage{"report.op": json.RawMessage(`1`)}},
		"events":           {Events: []string{"exec.started"}},
		"text":             {TextContains: "absent"},
	}
	for name, exp := range cases {
		t.Run(name, func(t *testing.T) {
			if problems := runCase(ctx, c, bindingCase{ID: name, Call: call, Expect: exp}); len(problems) == 0 {
				t.Fatal("mismatch was not detected")
			}
		})
	}
}

func TestCaseLoaderRejectsIncompleteCase(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "bad.json"), []byte(`{"id":"x"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadCases(dir); err == nil {
		t.Fatal("case without call or steps was accepted")
	}
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
