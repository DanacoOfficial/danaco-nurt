// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/test/locate.test.mjs
// Responsibility:   Source file bindings/node/test/locate.test.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import assert from "node:assert/strict";
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, join, normalize } from "node:path";
import { after, before, describe, it } from "node:test";
import { internal } from "./support.mjs";

const { locateExecutable } = internal("locate");
const { detectHost, executableName } = internal("platform");

const EXECUTABLE_MODE = 0o755;
const LINUX_GLIBC = { platform: "linux", arch: "x64", libc: "glibc" };

let root;

/** Creates an executable file and returns its path. */
function executable(...parts) {
  const file = join(root, ...parts);
  mkdirSync(join(file, ".."), { recursive: true });
  writeFileSync(file, "");
  chmodSync(file, EXECUTABLE_MODE);
  return file;
}

/** Resolver of installed packages backed by a map of package directories. */
function packages(installed) {
  return (name) => installed[name];
}

/** @returns {any} the error thrown by `locateExecutable` */
function failure(options) {
  try {
    locateExecutable(options);
  } catch (error) {
    return error;
  }
  assert.fail("locateExecutable did not fail");
}

describe("locateExecutable", () => {
  before(() => {
    root = mkdtempSync(join(tmpdir(), "nurt-locate-"));
  });

  after(() => {
    rmSync(root, { recursive: true, force: true });
  });

  it("returns the executable of the installed platform package", () => {
    const file = executable("modules", "nurt-linux-x64-gnu", "bin", "nurt");
    const found = locateExecutable({
      env: {},
      includePath: false,
      host: LINUX_GLIBC,
      resolvePackage: packages({ "nurt-linux-x64-gnu": join(root, "modules", "nurt-linux-x64-gnu") }),
    });
    assert.equal(normalize(found), normalize(file));
  });

  it("falls back to the package of the other C library", () => {
    const file = executable("fallback", "nurt-linux-x64-musl", "bin", "nurt");
    const found = locateExecutable({
      env: {},
      includePath: false,
      host: LINUX_GLIBC,
      resolvePackage: packages({ "nurt-linux-x64-musl": join(root, "fallback", "nurt-linux-x64-musl") }),
    });
    assert.equal(normalize(found), normalize(file));
  });

  it("prefers the executable option over the variable and the packages", () => {
    const option = executable("option", executableName(process.platform));
    const variable = executable("variable", executableName(process.platform));
    const found = locateExecutable({ executable: option, env: { NURT_EXECUTABLE: variable }, includePath: true });
    assert.equal(found, option);
  });

  it("uses NURT_EXECUTABLE before the packages", () => {
    const variable = executable("variable-only", executableName(process.platform));
    const found = locateExecutable({
      env: { NURT_EXECUTABLE: variable },
      includePath: false,
      resolvePackage: () => assert.fail("packages are not consulted"),
    });
    assert.equal(found, variable);
  });

  it("fails without fallback when the given path is not an executable file", () => {
    const missing = join(root, "missing", "nurt");
    const error = failure({ executable: missing, env: {}, includePath: true });
    assert.equal(error.name, "NurtError");
    assert.equal(error.status, "NURT_E_IO");
    assert.match(error.message, /the executable option names .*missing/);
  });

  it("searches PATH and skips node_modules/.bin", () => {
    const name = executableName(process.platform);
    executable("project", "node_modules", ".bin", name);
    const tool = executable("tools", name);
    const found = locateExecutable({
      env: { PATH: [join(root, "project", "node_modules", ".bin"), join(root, "tools")].join(delimiter) },
      includePath: true,
      host: detectHost(),
      resolvePackage: () => undefined,
    });
    assert.equal(found, tool);
  });

  it("does not search PATH when includePath is false", () => {
    const name = executableName(process.platform);
    executable("unused", name);
    const error = failure({
      env: { PATH: join(root, "unused") },
      includePath: false,
      host: detectHost(),
      resolvePackage: () => undefined,
    });
    assert.equal(error.status, "NURT_E_IO");
  });

  it("lists every place checked when nothing is found", () => {
    const error = failure({
      env: { PATH: join(root, "empty") },
      includePath: true,
      host: LINUX_GLIBC,
      resolvePackage: () => undefined,
    });
    assert.equal(error.status, "NURT_E_IO");
    assert.match(error.message, /package nurt-linux-x64-gnu is not installed/);
    assert.match(error.message, /package nurt-linux-x64-musl is not installed/);
    assert.match(error.message, /nurt is not in PATH/);
    assert.match(error.message, /NURT_EXECUTABLE/);
  });

  it("reports an unsupported platform", () => {
    const error = failure({ env: {}, includePath: false, host: { platform: "aix", arch: "ppc64" } });
    assert.match(error.message, /no platform package exists for aix-ppc64/);
  });
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
