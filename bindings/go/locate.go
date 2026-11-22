// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl

package nurt

import (
	"os"
	"os/exec"
)

// ExecutableEnv is the environment variable that names the nurt executable
// when ClientOptions.Executable is empty.
const ExecutableEnv = "NURT_EXECUTABLE"

const executableName = "nurt"

// The functions of the host system are replaceable so that the search order
// can be tested without touching the environment of the process.
type host struct {
	getenv   func(string) string
	lookPath func(string) (string, error)
	stat     func(string) (os.FileInfo, error)
}

var systemHost = host{getenv: os.Getenv, lookPath: exec.LookPath, stat: os.Stat}

// The search order is ClientOptions.Executable, then NURT_EXECUTABLE, then PATH.
// An explicit path that does not name a file is an error and the search
// does not continue, so that a misconfigured path is never replaced by
// another executable.
func findExecutable(explicit string, h host) (string, error) {
	isFile := func(p string) bool {
		fi, err := h.stat(p)
		return err == nil && !fi.IsDir()
	}
	if explicit != "" {
		if isFile(explicit) {
			return explicit, nil
		}
		return "", notFound("ClientOptions.Executable " + explicit)
	}
	if env := h.getenv(ExecutableEnv); env != "" {
		if isFile(env) {
			return env, nil
		}
		return "", notFound(ExecutableEnv + "=" + env)
	}
	if p, err := h.lookPath(executableName); err == nil {
		return p, nil
	}
	return "", notFound("ClientOptions.Executable (empty)", ExecutableEnv+" (unset)", "PATH")
}

func notFound(checked ...string) *Error {
	e := newError(StatusIO, "cannot find the nurt executable; set ClientOptions.Executable or "+ExecutableEnv+", or add nurt to PATH", nil)
	e.Checked = checked
	return e
}
