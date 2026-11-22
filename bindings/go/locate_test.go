// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"errors"
	"io/fs"
	"os"
	"strings"
	"testing"
	"time"
)

type fakeFile struct {
	dir bool
}

func (f fakeFile) Name() string       { return "nurt" }
func (f fakeFile) Size() int64        { return 0 }
func (f fakeFile) Mode() fs.FileMode  { return 0o755 }
func (f fakeFile) ModTime() time.Time { return time.Time{} }
func (f fakeFile) IsDir() bool        { return f.dir }
func (f fakeFile) Sys() any           { return nil }

// The host has the given files (true marks a directory), environment and
// result of the PATH search for nurt.
func testHost(files map[string]bool, env map[string]string, onPath string) host {
	return host{
		getenv: func(k string) string { return env[k] },
		lookPath: func(name string) (string, error) {
			if onPath == "" {
				return "", errors.New("not found")
			}
			return onPath, nil
		},
		stat: func(p string) (os.FileInfo, error) {
			dir, ok := files[p]
			if !ok {
				return nil, fs.ErrNotExist
			}
			return fakeFile{dir: dir}, nil
		},
	}
}

func TestExplicitExecutableTakesPrecedence(t *testing.T) {
	h := testHost(map[string]bool{"/opt/nurt": false, "/env/nurt": false},
		map[string]string{ExecutableEnv: "/env/nurt"}, "/usr/bin/nurt")
	got, err := findExecutable("/opt/nurt", h)
	if err != nil || got != "/opt/nurt" {
		t.Fatalf("findExecutable = %q, %v", got, err)
	}
}

func TestEnvironmentVariableComesBeforePath(t *testing.T) {
	h := testHost(map[string]bool{"/env/nurt": false}, map[string]string{ExecutableEnv: "/env/nurt"}, "/usr/bin/nurt")
	got, err := findExecutable("", h)
	if err != nil || got != "/env/nurt" {
		t.Fatalf("findExecutable = %q, %v", got, err)
	}
}

func TestPathIsSearchedLast(t *testing.T) {
	got, err := findExecutable("", testHost(nil, nil, "/usr/bin/nurt"))
	if err != nil || got != "/usr/bin/nurt" {
		t.Fatalf("findExecutable = %q, %v", got, err)
	}
}

func TestInvalidExplicitLocationDoesNotFallBack(t *testing.T) {
	cases := map[string]struct {
		explicit string
		env      map[string]string
		files    map[string]bool
		mention  string
	}{
		"missing option":      {explicit: "/opt/missing", mention: "/opt/missing"},
		"directory option":    {explicit: "/opt", files: map[string]bool{"/opt": true}, mention: "/opt"},
		"missing environment": {env: map[string]string{ExecutableEnv: "/env/missing"}, mention: ExecutableEnv + "=/env/missing"},
	}
	for name, tc := range cases {
		t.Run(name, func(t *testing.T) {
			_, err := findExecutable(tc.explicit, testHost(tc.files, tc.env, "/usr/bin/nurt"))
			e := wantStatus(t, err, StatusIO)
			if !strings.Contains(e.Error(), tc.mention) {
				t.Fatalf("error %q does not mention %q", e.Error(), tc.mention)
			}
		})
	}
}

func TestNotFoundListsEveryCheckedLocation(t *testing.T) {
	_, err := findExecutable("", testHost(nil, nil, ""))
	e := wantStatus(t, err, StatusIO)
	if len(e.Checked) != 3 {
		t.Fatalf("checked = %v, want option, environment variable and PATH", e.Checked)
	}
	for _, want := range []string{"ClientOptions.Executable", ExecutableEnv, "PATH"} {
		if !strings.Contains(e.Error(), want) {
			t.Errorf("error %q does not mention %q", e.Error(), want)
		}
	}
}
