// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"encoding/json"
	"errors"
	"io/fs"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"testing"
)

// The variable names the directory of the published JSON schemas. By
// default the schemas are read from the repository that contains the module.
const specDirEnv = "NURT_SPEC_DIR"

var defaultSpecDir = filepath.Join("..", "..", "spec", "schemas")

type schemaNode struct {
	Properties map[string]*schemaNode `json:"properties"`
	Defs       map[string]*schemaNode `json:"$defs"`
	Enum       []string               `json:"enum"`
}

func loadSchema(t *testing.T, name string) *schemaNode {
	t.Helper()
	dir := os.Getenv(specDirEnv)
	if dir == "" {
		dir = defaultSpecDir
	}
	raw, err := os.ReadFile(filepath.Join(dir, name))
	if errors.Is(err, fs.ErrNotExist) {
		t.Skipf("schema %s not found in %s; set %s", name, dir, specDirEnv)
	}
	if err != nil {
		t.Fatal(err)
	}
	var s schemaNode
	if err := json.Unmarshal(raw, &s); err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	return &s
}

func jsonNames(v any) []string {
	var names []string
	rt := reflect.TypeOf(v)
	for i := range rt.NumField() {
		f := rt.Field(i)
		tag := f.Tag.Get("json")
		name, _, _ := strings.Cut(tag, ",")
		if !f.IsExported() || name == "-" || name == "" {
			continue
		}
		names = append(names, name)
	}
	return names
}

func keys(m map[string]*schemaNode) []string {
	var out []string
	for k := range m {
		out = append(out, k)
	}
	slices.Sort(out)
	return out
}

func sorted(s []string) []string {
	out := slices.Clone(s)
	slices.Sort(out)
	return out
}

func expectSame(t *testing.T, what string, got, want []string) {
	t.Helper()
	if !slices.Equal(sorted(got), sorted(want)) {
		t.Errorf("%s: binding has %v, schema has %v", what, sorted(got), sorted(want))
	}
}

func expectSubset(t *testing.T, what string, got []string, schema map[string]*schemaNode) {
	t.Helper()
	for _, name := range got {
		if _, ok := schema[name]; !ok {
			t.Errorf("%s: field %q is not in the schema", what, name)
		}
	}
}

func TestRequestTypesMatchSchema(t *testing.T) {
	s := loadSchema(t, "nurt-request-1.schema.json")
	expectSame(t, "request envelope", append(jsonNames(Request{}), "nurt"), keys(s.Properties))
	expectSame(t, "options", append(jsonNames(Options{}), "timeout"), keys(s.Defs["options"].Properties))
	expectSame(t, "variant", []string{string(VariantA), string(VariantB), string(VariantC)}, s.Properties["variant"].Enum)
	expectSame(t, "format", []string{string(FormatText), string(FormatJSON), string(FormatMarkdown), string(FormatCompact)}, s.Defs["format"].Enum)
	opts := s.Defs["options"].Properties
	expectSame(t, "verbosity", []string{string(VerbosityQuiet), string(VerbosityNormal), string(VerbosityDetailed)}, opts["verbosity"].Enum)
	expectSame(t, "diff", []string{string(DiffNone), string(DiffStat), string(DiffUnified)}, opts["diff"].Enum)
	expectSame(t, "response", jsonNames(Response{}), keys(s.Defs["response"].Properties))
	expectSame(t, "rendered", jsonNames(Rendered{}), keys(s.Defs["rendered"].Properties))
	expectSame(t, "rendered format", []string{string(FormatText), string(FormatMarkdown), string(FormatCompact)}, s.Defs["rendered"].Properties["format"].Enum)
}

func TestReportTypesMatchSchema(t *testing.T) {
	s := loadSchema(t, "nurt-report-1.schema.json")
	expectSubset(t, "report", jsonNames(Report{}), s.Properties)
	errSchema := s.Defs["error"]
	expectSubset(t, "language error", jsonNames(LanguageError{}), errSchema.Properties)
	expectSubset(t, "position", jsonNames(Position{}), errSchema.Properties["at"].Properties)
	statuses := []string{
		string(ReportOK), string(ReportPlanned), string(ReportRunning),
		string(ReportPartial), string(ReportRolledBack), string(ReportFailed),
	}
	expectSame(t, "report status", statuses, s.Defs["status"].Enum)
}
