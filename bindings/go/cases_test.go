// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/go/cases_test.go
// Responsibility:   Source file bindings/go/cases_test.go.
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
	"regexp"
	"runtime"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"
)

// The runner of the binding cases of the repository, conformance/bindings/*.json
// in the format nurt-binding-cases/1. Each step calls the API of the client and
// the outcome is compared with the expectation through JSON pointers into the
// result view of the call.
const (
	bindingCasesFormat = "nurt-binding-cases/1"
	caseNameVariable   = "NURT_CONFORMANCE_CASE"
	casesRootVariable  = "NURT_CONFORMANCE_CASES"
	stepTimeout        = 30 * time.Second

	outcomeOK       = "ok"
	outcomeLanguage = "language_error"
	outcomeCall     = "call_error"
)

type bindingCasesFile struct {
	Format string        `json:"format"`
	Cases  []repoCaseDef `json:"cases"`
}

type repoCaseDef struct {
	ID       string        `json:"id"`
	Class    string        `json:"class"`
	Requires []string      `json:"requires"`
	Backends []Backend     `json:"backends"`
	FS       []caseFSEntry `json:"fs"`
	Steps    []repoStepDef `json:"steps"`
}

type caseFSEntry struct {
	Path    string `json:"path"`
	Type    string `json:"type"`
	Target  string `json:"target"`
	Content string `json:"content_utf8"`
}

type repoStepDef struct {
	Call      string          `json:"call"`
	Execution int             `json:"execution"`
	Args      json.RawMessage `json:"args"`
	Expect    repoExpectation `json:"expect"`
}

type repoExpectation struct {
	Outcome string                     `json:"outcome"`
	Status  string                     `json:"status"`
	Code    string                     `json:"code"`
	Fields  map[string]json.RawMessage `json:"fields"`
	Present []string                   `json:"present"`
	Absent  []string                   `json:"absent"`
}

// The outcome of a step: its kind, the view that the pointers address, the
// status name of a call error and the code of a language error.
type repoOutcome struct {
	kind   string
	view   map[string]any
	status string
	code   string
}

func loadRepoCases(dir string) ([]repoCaseDef, error) {
	paths, err := filepath.Glob(filepath.Join(dir, "*.json"))
	if err != nil {
		return nil, err
	}
	if _, err := os.Stat(dir); err != nil {
		return nil, err
	}
	slices.Sort(paths)
	var cases []repoCaseDef
	for _, p := range paths {
		raw, err := os.ReadFile(p)
		if err != nil {
			return nil, err
		}
		found, err := parseRepoCases(raw, p)
		if err != nil {
			return nil, err
		}
		cases = append(cases, found...)
	}
	return cases, nil
}

func parseRepoCases(raw []byte, name string) ([]repoCaseDef, error) {
	var file bindingCasesFile
	if err := json.Unmarshal(raw, &file); err != nil {
		return nil, fmt.Errorf("%s: %w", name, err)
	}
	if file.Format != bindingCasesFormat {
		return nil, fmt.Errorf("%s is not in the format %s", name, bindingCasesFormat)
	}
	return file.Cases, nil
}

var placeholderPattern = regexp.MustCompile(`\$\{(base|step(\d+)(/[^}]*))\}`)

// Replaces ${base} and ${stepN/pointer} against the views of earlier steps.
type placeholders struct {
	base  string
	views []map[string]any
}

func (p *placeholders) substitute(value any) (any, error) {
	switch v := value.(type) {
	case string:
		if loc := placeholderPattern.FindStringSubmatchIndex(v); loc != nil && loc[0] == 0 && loc[1] == len(v) {
			return p.resolve(placeholderPattern.FindStringSubmatch(v))
		}
		var failure error
		replaced := placeholderPattern.ReplaceAllStringFunc(v, func(match string) string {
			found, err := p.resolve(placeholderPattern.FindStringSubmatch(match))
			if err != nil {
				failure = err
				return match
			}
			return fmt.Sprint(found)
		})
		return replaced, failure
	case []any:
		out := make([]any, len(v))
		for i, item := range v {
			next, err := p.substitute(item)
			if err != nil {
				return nil, err
			}
			out[i] = next
		}
		return out, nil
	case map[string]any:
		out := make(map[string]any, len(v))
		for key, item := range v {
			next, err := p.substitute(item)
			if err != nil {
				return nil, err
			}
			out[key] = next
		}
		return out, nil
	}
	return value, nil
}

func (p *placeholders) resolve(match []string) (any, error) {
	if match[1] == "base" {
		if p.base == "" {
			return nil, errors.New("${base} is used by a case without fs")
		}
		return p.base, nil
	}
	number, _ := strconv.Atoi(match[2])
	if number < 1 || number > len(p.views) {
		return nil, fmt.Errorf("step %d has not run", number)
	}
	found, ok := pointerGet(p.views[number-1], match[3])
	if !ok {
		return nil, fmt.Errorf("the result of step %d has no %s", number, match[3])
	}
	return found, nil
}

// Resolves a JSON pointer; the empty pointer addresses the document.
func pointerGet(document any, pointer string) (any, bool) {
	if pointer == "" {
		return document, true
	}
	target := document
	for _, raw := range strings.Split(pointer, "/")[1:] {
		part := strings.ReplaceAll(strings.ReplaceAll(raw, "~1", "/"), "~0", "~")
		switch node := target.(type) {
		case map[string]any:
			next, ok := node[part]
			if !ok {
				return nil, false
			}
			target = next
		case []any:
			index, err := strconv.Atoi(part)
			if err != nil || index < 0 || index >= len(node) {
				return nil, false
			}
			target = node[index]
		default:
			return nil, false
		}
	}
	return target, true
}

func materializeFS(t *testing.T, entries []caseFSEntry, root string) {
	t.Helper()
	for _, entry := range entries {
		path := filepath.Join(root, filepath.FromSlash(entry.Path))
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			t.Fatal(err)
		}
		var err error
		switch entry.Type {
		case "dir":
			err = os.MkdirAll(path, 0o755)
		case "symlink":
			err = os.Symlink(entry.Target, path)
		default:
			err = os.WriteFile(path, []byte(entry.Content), 0o644)
		}
		if err != nil {
			t.Fatal(err)
		}
	}
}

// The view of a response: ok, report, text, error and next_cursor.
func responseView(resp *Response) map[string]any {
	view := map[string]any{"ok": resp.OK()}
	var raw map[string]any
	if json.Unmarshal(resp.Raw, &raw) != nil {
		return view
	}
	if report, ok := raw["report"].(map[string]any); ok {
		view["report"] = report
		if failure, ok := report["error"].(map[string]any); ok {
			view["error"] = failure
		}
		if cursor, ok := report["next_cursor"].(string); ok {
			view["next_cursor"] = cursor
		}
	}
	if rendered, ok := raw["rendered"].(map[string]any); ok {
		if text, ok := rendered["text"].(string); ok {
			view["text"] = text
		}
	}
	return view
}

func decodeAny(raw []byte) any {
	var v any
	if json.Unmarshal(raw, &v) != nil {
		return nil
	}
	return v
}

// Splits the arguments of run, start and check into a request; every
// argument that is not a member of the envelope is an option.
func requestFromArgs(args map[string]any) (Request, error) {
	var req Request
	options := map[string]any{}
	if given, ok := args["options"].(map[string]any); ok {
		for key, value := range given {
			options[key] = value
		}
	}
	for key, value := range args {
		text, _ := value.(string)
		switch key {
		case "options", "raise_on_error":
		case "program":
			req.Program = text
		case "variant":
			req.Variant = Variant(text)
		case "cursor":
			req.Cursor = text
		case "accept":
			req.Accept = text
		case "ast":
			raw, err := json.Marshal(value)
			if err != nil {
				return req, err
			}
			req.AST = raw
		default:
			options[key] = value
		}
	}
	var timeout time.Duration
	if text, ok := options["timeout"].(string); ok {
		parsed, err := time.ParseDuration(text)
		if err != nil {
			return req, fmt.Errorf("timeout %q: %w", text, err)
		}
		timeout = parsed
		delete(options, "timeout")
	}
	raw, err := json.Marshal(options)
	if err != nil {
		return req, err
	}
	if err := json.Unmarshal(raw, &req.Options); err != nil {
		return req, fmt.Errorf("options: %w", err)
	}
	req.Options.Timeout = timeout
	req.FailOnLanguageError, _ = args["raise_on_error"].(bool)
	return req, nil
}

type repoRunner struct {
	ctx        context.Context
	client     *Client
	executions map[int]*Execution
}

func argString(args map[string]any, name string) string {
	text, _ := args[name].(string)
	return text
}

func argRaw(args map[string]any, name string) json.RawMessage {
	value, ok := args[name]
	if !ok || value == nil {
		return nil
	}
	raw, _ := json.Marshal(value)
	return raw
}

func valueView(raw []byte) map[string]any { return map[string]any{"value": decodeAny(raw)} }

func (r *repoRunner) perform(number int, step repoStepDef, args map[string]any) repoOutcome {
	view, err := r.call(number, step, args)
	return classify(step.Call, view, err)
}

func (r *repoRunner) call(number int, step repoStepDef, args map[string]any) (map[string]any, error) {
	c, ctx := r.client, r.ctx
	switch step.Call {
	case "run":
		req, err := requestFromArgs(args)
		if err != nil {
			return nil, err
		}
		resp, err := c.Run(ctx, req)
		return viewOf(resp), err
	case "start":
		req, err := requestFromArgs(args)
		if err != nil {
			return nil, err
		}
		x, err := c.Start(ctx, req)
		if err == nil {
			r.executions[number] = x
		}
		return map[string]any{}, err
	case "check":
		req, err := requestFromArgs(args)
		if err != nil {
			return nil, err
		}
		res, err := c.Check(ctx, req)
		if err != nil {
			return nil, err
		}
		return valueView(res.Raw), nil
	case "parse":
		res, err := c.Parse(ctx, argString(args, "program"), Variant(argString(args, "variant")))
		if err != nil {
			return nil, err
		}
		return valueView(res.Raw), nil
	case "print":
		text, err := c.Print(ctx, argRaw(args, "ast"), Variant(argString(args, "variant")))
		return map[string]any{"value": text}, err
	case "correct":
		res, err := c.Correct(ctx, argString(args, "program"), Variant(argString(args, "variant")))
		if err != nil {
			return nil, err
		}
		return valueView(res.Raw), nil
	case "page":
		limit, _ := args["limit"].(float64)
		resp, err := c.Page(ctx, argString(args, "cursor"), int(limit))
		return viewOf(resp), err
	case "describe":
		layer := argString(args, "layer")
		if layer == "" {
			layer = "core"
		}
		text, err := c.Describe(ctx, DescribeQuery{Layer: layer, Name: argString(args, "name"), Language: argString(args, "language")})
		return map[string]any{"value": text}, err
	case "help":
		text, err := c.Help(ctx, argString(args, "topic"))
		return map[string]any{"value": text}, err
	case "catalog":
		return rawView(c.Catalog(ctx, argRaw(args, "what")))
	case "schema":
		return rawView(c.Schema(ctx, argString(args, "which")))
	case "history":
		return rawView(c.History(ctx, argRaw(args, "query")))
	case "space":
		return rawView(c.Space(ctx, argString(args, "volume")))
	case "info":
		return rawView(c.Info(ctx))
	case "close":
		return map[string]any{}, c.Close()
	case "wait", "cancel", "events":
		x, ok := r.executions[step.Execution]
		if !ok {
			return nil, fmt.Errorf("step %d has started no execution", step.Execution)
		}
		return r.execution(x, step.Call, args)
	}
	return nil, fmt.Errorf("unknown call %q", step.Call)
}

func viewOf(resp *Response) map[string]any {
	if resp == nil {
		return map[string]any{}
	}
	return responseView(resp)
}

func rawView(raw json.RawMessage, err error) (map[string]any, error) {
	if err != nil {
		return nil, err
	}
	return valueView(raw), nil
}

func (r *repoRunner) execution(x *Execution, call string, args map[string]any) (map[string]any, error) {
	switch call {
	case "cancel":
		return map[string]any{}, x.Cancel(r.ctx)
	case "wait":
		timeout := stepTimeout
		if ms, ok := args["timeout_ms"].(float64); ok {
			timeout = time.Duration(ms * float64(time.Millisecond))
		}
		ctx, cancel := context.WithTimeout(r.ctx, timeout)
		defer cancel()
		resp, err := x.Wait(ctx)
		var failure *Error
		if resp != nil && errors.As(err, &failure) && failure.Status == StatusCancelled {
			err = nil
		}
		return viewOf(resp), err
	}
	ctx, cancel := context.WithTimeout(r.ctx, stepTimeout)
	defer cancel()
	if _, err := x.Wait(ctx); err != nil {
		_ = x.Close()
	}
	events := []any{}
	for ev := range x.Events() {
		events = append(events, decodeAny(ev.Raw))
	}
	return map[string]any{"value": events}, nil
}

// Sorts the result of a call into ok, a language error or a call error.
func classify(call string, view map[string]any, err error) repoOutcome {
	if view == nil {
		view = map[string]any{}
	}
	var language *LanguageError
	var failure *Error
	switch {
	case errors.As(err, &language):
		return repoOutcome{kind: outcomeLanguage, view: view, code: language.Code}
	case errors.As(err, &failure):
		return repoOutcome{kind: outcomeCall, view: map[string]any{}, status: failure.Status.String()}
	case err != nil:
		return repoOutcome{kind: outcomeCall, view: map[string]any{}, status: StatusInternal.String()}
	}
	if slices.Contains([]string{"run", "page", "wait"}, call) && view["ok"] == false {
		code, _ := pointerGet(view, "/error/code")
		text, _ := code.(string)
		return repoOutcome{kind: outcomeLanguage, view: view, code: text}
	}
	for _, pointer := range []string{"/value/error/code", "/value/report/error/code"} {
		if code, ok := pointerGet(view, pointer); ok {
			text, _ := code.(string)
			return repoOutcome{kind: outcomeLanguage, view: view, code: text}
		}
	}
	return repoOutcome{kind: outcomeOK, view: view}
}

func unmetExpectations(expect repoExpectation, got repoOutcome) []string {
	var found []string
	if got.kind != expect.Outcome {
		found = append(found, fmt.Sprintf("outcome %s, expected %s (status %q, code %q)", got.kind, expect.Outcome, got.status, got.code))
	}
	if expect.Status != "" && got.status != expect.Status {
		found = append(found, fmt.Sprintf("status %q, expected %q", got.status, expect.Status))
	}
	if expect.Code != "" && got.code != expect.Code {
		found = append(found, fmt.Sprintf("code %q, expected %q", got.code, expect.Code))
	}
	for _, pointer := range slices.Sorted(maps.Keys(expect.Fields)) {
		var want any
		if err := json.Unmarshal(expect.Fields[pointer], &want); err != nil {
			found = append(found, fmt.Sprintf("expected value of %s is not JSON: %v", pointer, err))
			continue
		}
		actual, ok := pointerGet(got.view, pointer)
		switch {
		case !ok:
			found = append(found, pointer+" is missing")
		case !reflect.DeepEqual(actual, want):
			found = append(found, fmt.Sprintf("%s is %v, expected %v", pointer, actual, want))
		}
	}
	for _, pointer := range expect.Present {
		if _, ok := pointerGet(got.view, pointer); !ok {
			found = append(found, pointer+" is missing")
		}
	}
	for _, pointer := range expect.Absent {
		if _, ok := pointerGet(got.view, pointer); ok {
			found = append(found, pointer+" is present")
		}
	}
	return found
}

// The reason a case does not apply to the backend or the system, or "".
func unmetRequirement(ctx context.Context, c *Client, tc repoCaseDef, backend Backend) string {
	if len(tc.Backends) > 0 && !slices.Contains(tc.Backends, backend) {
		return "the case is not for the " + string(backend) + " backend"
	}
	for _, condition := range tc.Requires {
		kind, value, _ := strings.Cut(condition, ":")
		if kind == "os" && (value == "windows") != (runtime.GOOS == "windows") {
			return "the case requires " + condition
		}
	}
	if feature, ok := strings.CutPrefix(tc.Class, optionalPrefix); ok && !hasFeature(ctx, c, feature) {
		return "the implementation does not declare the feature " + feature
	}
	return ""
}

// Runs the steps of a case and returns the unmet expectations, prefixed with
// the number of the step.
func runRepoCase(ctx context.Context, t *testing.T, c *Client, tc repoCaseDef) []string {
	t.Helper()
	holder := &placeholders{}
	if len(tc.FS) > 0 {
		holder.base = t.TempDir()
		materializeFS(t, tc.FS, holder.base)
	}
	runner := &repoRunner{ctx: ctx, client: c, executions: map[int]*Execution{}}
	defer func() {
		for _, x := range runner.executions {
			_ = x.Close()
		}
	}()
	var found []string
	for number, step := range tc.Steps {
		number++
		var args map[string]any
		if len(step.Args) > 0 {
			if err := json.Unmarshal(step.Args, &args); err != nil {
				return append(found, fmt.Sprintf("step %d: arguments are not an object: %v", number, err))
			}
		}
		substituted, err := holder.substitute(args)
		if err != nil {
			return append(found, fmt.Sprintf("step %d: %v", number, err))
		}
		args, _ = substituted.(map[string]any)
		if args == nil {
			args = map[string]any{}
		}
		got := runner.perform(number, step, args)
		holder.views = append(holder.views, got.view)
		for _, item := range unmetExpectations(step.Expect, got) {
			found = append(found, fmt.Sprintf("step %d (%s): %s", number, step.Call, item))
		}
	}
	return found
}

func runRepoCases(t *testing.T, cases []repoCaseDef, backends []Backend, newClient func(*testing.T, Backend, string) *Client) {
	t.Helper()
	for _, b := range backends {
		for _, tc := range cases {
			t.Run(string(b)+"/"+tc.ID, func(t *testing.T) {
				ctx, cancel := context.WithTimeout(context.Background(), caseTimeout)
				defer cancel()
				c := newClient(t, b, tc.ID)
				if reason := unmetRequirement(ctx, c, tc, b); reason != "" {
					t.Skip(reason)
				}
				for _, problem := range runRepoCase(ctx, t, c, tc) {
					t.Error(problem)
				}
			})
		}
	}
}

// The cases of the repository run against the implementation named by
// NURT_TEST_COMMAND, a JSON array with the command that starts nurt serve
// --stdio, and against libnurt when the module is built with nurt_cgo.
func TestConformanceBindings(t *testing.T) {
	dir := os.Getenv(conformanceDirEnv)
	if dir == "" {
		dir = defaultConformanceDir
	}
	cases, err := loadRepoCases(dir)
	if errors.Is(err, os.ErrNotExist) {
		t.Skipf("binding cases not found in %s; set %s", dir, conformanceDirEnv)
	}
	if err != nil {
		t.Fatal(err)
	}
	var command []string
	if raw := os.Getenv(testCommandEnv); raw != "" {
		if err := json.Unmarshal([]byte(raw), &command); err != nil || len(command) == 0 {
			t.Fatalf("%s must be a non-empty JSON array of strings", testCommandEnv)
		}
	}
	var backends []Backend
	for _, b := range conformanceBackends {
		if b != BackendProcess || command != nil {
			backends = append(backends, b)
		}
	}
	if len(backends) == 0 {
		t.Skipf("no implementation to test; set %s", testCommandEnv)
	}
	root := dir
	if filepath.Base(dir) == "bindings" {
		root = filepath.Dir(dir)
	}
	runRepoCases(t, cases, backends, func(t *testing.T, b Backend, id string) *Client {
		stateDir := t.TempDir()
		opts := ClientOptions{Backend: b}
		if b == BackendProcess {
			opts.Executable, opts.Args = command[0], command[1:]
			opts.Env = []string{"NURT_STATE_DIR=" + stateDir, caseNameVariable + "=" + id, casesRootVariable + "=" + root}
		} else {
			t.Setenv("NURT_STATE_DIR", stateDir)
		}
		return newTestClient(t, opts)
	})
}

const fakeBindingCases = `{"format":"nurt-binding-cases/1","cases":[
{"id":"run.ok","steps":[{"call":"run","args":{"program":"info","options":{"limit":5}},"expect":{"outcome":"ok","fields":{"/ok":true,"/report/status":"ok","/report/kind":"run"},"present":["/text"],"absent":["/error"]}}]},
{"id":"run.language-error","steps":[{"call":"run","args":{"program":"error"},"expect":{"outcome":"language_error","code":"E_SYNTAX","fields":{"/ok":false}}}]},
{"id":"run.raise-on-error","steps":[{"call":"run","args":{"program":"error","raise_on_error":true},"expect":{"outcome":"language_error","code":"E_SYNTAX"}}]},
{"id":"page.follows-cursor","steps":[
 {"call":"run","args":{"program":"paged"},"expect":{"outcome":"ok","fields":{"/next_cursor":"c1"}}},
 {"call":"page","args":{"cursor":"${step1/next_cursor}"},"expect":{"outcome":"ok","fields":{"/text":"page c1"}}}]},
{"id":"start.events","steps":[
 {"call":"start","args":{"program":"events"},"expect":{"outcome":"ok"}},
 {"call":"events","execution":1,"expect":{"outcome":"ok","fields":{"/value/0/t":"exec.started","/value/2/t":"exec.finished"}}},
 {"call":"wait","execution":1,"expect":{"outcome":"ok","fields":{"/ok":true}}}]},
{"id":"start.cancel","steps":[
 {"call":"start","args":{"program":"sleep"},"expect":{"outcome":"ok"}},
 {"call":"cancel","execution":1,"expect":{"outcome":"ok"}},
 {"call":"wait","execution":1,"args":{"timeout_ms":10000},"expect":{"outcome":"ok","fields":{"/report/status":"rolled_back"}}}]},
{"id":"schema.unknown","steps":[{"call":"schema","args":{"which":"unknown"},"expect":{"outcome":"call_error","status":"NURT_E_ARG"}}]},
{"id":"parse.value","steps":[{"call":"parse","args":{"program":"info"},"expect":{"outcome":"ok","fields":{"/value/ast/nurt":"1.0"},"absent":["/value/error"]}}]},
{"id":"check.value","steps":[{"call":"check","args":{"program":"info"},"expect":{"outcome":"ok","fields":{"/value/bound/nurt":"1.0"}}}]},
{"id":"describe.text","steps":[{"call":"describe","args":{"layer":"core","language":"en"},"expect":{"outcome":"ok","fields":{"/value":"describe core en"}}}]},
{"id":"info.then-close","steps":[
 {"call":"info","expect":{"outcome":"ok","fields":{"/value/version":"0.0.0"}}},
 {"call":"close","expect":{"outcome":"ok"}},
 {"call":"run","args":{"program":"info"},"expect":{"outcome":"call_error","status":"NURT_E_ARG"}}]},
{"id":"other.backend","backends":["library"],"steps":[{"call":"run","args":{"program":"info"},"expect":{"outcome":"call_error"}}]}
]}`

func TestRepositoryCasesRunnerOnFakeServer(t *testing.T) {
	cases, err := parseRepoCases([]byte(fakeBindingCases), "fakeBindingCases")
	if err != nil {
		t.Fatal(err)
	}
	runRepoCases(t, cases, []Backend{BackendProcess}, func(t *testing.T, _ Backend, _ string) *Client {
		return newTestClient(t, fakeOptions(t, ""))
	})
}

func TestRepositoryCasesRunnerDetectsMismatch(t *testing.T) {
	c := newTestClient(t, fakeOptions(t, ""))
	step := func(call, args string, expect repoExpectation) repoCaseDef {
		return repoCaseDef{ID: "x", Steps: []repoStepDef{{Call: call, Args: json.RawMessage(args), Expect: expect}}}
	}
	wrong := map[string]repoCaseDef{
		"outcome":    step("run", `{"program":"error"}`, repoExpectation{Outcome: outcomeOK}),
		"code":       step("run", `{"program":"error"}`, repoExpectation{Outcome: outcomeLanguage, Code: "E_LEXICAL"}),
		"status":     step("schema", `{"which":"unknown"}`, repoExpectation{Outcome: outcomeCall, Status: "NURT_E_IO"}),
		"field":      step("run", `{"program":"info"}`, repoExpectation{Outcome: outcomeOK, Fields: map[string]json.RawMessage{"/report/kind": json.RawMessage(`"find"`)}}),
		"present":    step("run", `{"program":"info"}`, repoExpectation{Outcome: outcomeOK, Present: []string{"/error"}}),
		"absent":     step("run", `{"program":"info"}`, repoExpectation{Outcome: outcomeOK, Absent: []string{"/report"}}),
		"unresolved": step("run", `{"program":"${step3/text}"}`, repoExpectation{Outcome: outcomeOK}),
	}
	for name, tc := range wrong {
		t.Run(name, func(t *testing.T) {
			if problems := runRepoCase(context.Background(), t, c, tc); len(problems) == 0 {
				t.Fatal("mismatch was not detected")
			}
		})
	}
}

func TestPointersAddressMembersAndItems(t *testing.T) {
	document := map[string]any{"value": []any{map[string]any{"t": "exec.started"}}, "a/b": map[string]any{"~": 1.0}}
	if got, ok := pointerGet(document, "/value/0/t"); !ok || got != "exec.started" {
		t.Errorf("/value/0/t = %v, %v", got, ok)
	}
	if got, ok := pointerGet(document, "/a~1b/~0"); !ok || got != 1.0 {
		t.Errorf("/a~1b/~0 = %v, %v", got, ok)
	}
	if _, ok := pointerGet(document, "/value/1"); ok {
		t.Error("/value/1 exists")
	}
	if _, ok := pointerGet(document, "/value/-1"); ok {
		t.Error("/value/-1 exists")
	}
	if got, ok := pointerGet(document, ""); !ok || !reflect.DeepEqual(got, document) {
		t.Error("the empty pointer does not address the document")
	}
}

func TestPlaceholdersTakeValuesOfEarlierSteps(t *testing.T) {
	holder := &placeholders{base: "/home/user/case", views: []map[string]any{{"next_cursor": "c-1", "limit": 2.0}}}
	got, err := holder.substitute(map[string]any{
		"cursor": "${step1/next_cursor}",
		"limit":  "${step1/limit}",
		"path":   "${base}/src",
		"list":   []any{"${base}"},
	})
	if err != nil {
		t.Fatal(err)
	}
	want := map[string]any{"cursor": "c-1", "limit": 2.0, "path": "/home/user/case/src", "list": []any{"/home/user/case"}}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("substituted %v, want %v", got, want)
	}
	if _, err := holder.substitute("${step2/next_cursor}"); err == nil {
		t.Error("a step that has not run was resolved")
	}
	if _, err := (&placeholders{}).substitute("${base}"); err == nil {
		t.Error("${base} was resolved without fs")
	}
}

func TestRequestArgumentsMoveEnvelopeFieldsAndOptions(t *testing.T) {
	req, err := requestFromArgs(map[string]any{
		"program":        "count",
		"cursor":         "c",
		"limit":          1.0,
		"options":        map[string]any{"timeout": "1500ms", "base": "/home/user"},
		"raise_on_error": true,
	})
	if err != nil {
		t.Fatal(err)
	}
	if req.Program != "count" || req.Cursor != "c" || !req.FailOnLanguageError {
		t.Errorf("request = %+v", req)
	}
	if req.Options.Limit != 1 || req.Options.Base != "/home/user" || req.Options.Timeout != 1500*time.Millisecond {
		t.Errorf("options = %+v", req.Options)
	}
	if _, err := requestFromArgs(map[string]any{"options": map[string]any{"timeout": "soon"}}); err == nil {
		t.Error("an invalid timeout was accepted")
	}
}

func TestLoaderRejectsFilesInAnotherFormat(t *testing.T) {
	dir := t.TempDir()
	if err := os.WriteFile(filepath.Join(dir, "old.json"), []byte(`{"id":"x","call":{"method":"run"}}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, err := loadRepoCases(dir); err == nil {
		t.Fatal("a file without the format member was accepted")
	}
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
