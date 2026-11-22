// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import * as assert from "assert";
import * as path from "path";
import {
  DEFAULT_EXECUTABLE,
  formatArguments,
  resolveExecutable,
  runArguments,
  serverArguments,
} from "../../src/executable";

suite("resolveExecutable", () => {
  test("uses the bare name when the setting is empty", () => {
    assert.strictEqual(resolveExecutable(""), DEFAULT_EXECUTABLE);
    assert.strictEqual(resolveExecutable("   "), DEFAULT_EXECUTABLE);
    assert.strictEqual(resolveExecutable(undefined), DEFAULT_EXECUTABLE);
  });

  test("keeps a bare name for lookup in the PATH", () => {
    assert.strictEqual(resolveExecutable("nurt"), "nurt");
  });

  test("keeps an absolute path unchanged", () => {
    assert.strictEqual(resolveExecutable("/opt/nurt/bin/nurt"), "/opt/nurt/bin/nurt");
  });

  test("expands a leading tilde to the home directory", () => {
    const expected = path.join("/home/user", "bin/nurt");
    assert.strictEqual(resolveExecutable("~/bin/nurt", { homeDirectory: "/home/user" }), expected);
    assert.strictEqual(resolveExecutable("~", { homeDirectory: "/home/user" }), path.join("/home/user", ""));
  });

  test("does not expand a tilde inside the path", () => {
    assert.strictEqual(resolveExecutable("/opt/a~b/nurt", { homeDirectory: "/home/user" }), "/opt/a~b/nurt");
  });

  test("expands the workspace folder variable", () => {
    assert.strictEqual(
      resolveExecutable("${workspaceFolder}/tools/nurt", { workspaceFolder: "/work/project" }),
      "/work/project/tools/nurt",
    );
  });

  test("rejects the workspace folder variable without an open folder", () => {
    assert.throws(() => resolveExecutable("${workspaceFolder}/nurt"), /no folder is open/);
  });
});

suite("command arguments", () => {
  test("starts the language server with the lsp subcommand", () => {
    assert.deepStrictEqual(serverArguments(), ["lsp"]);
  });

  test("runs a file with the script subcommand", () => {
    assert.deepStrictEqual(runArguments("/work/a.nurt"), ["script", "/work/a.nurt"]);
  });

  test("formats a file with the fmt subcommand", () => {
    assert.deepStrictEqual(formatArguments("/work/a.nurt"), ["fmt", "/work/a.nurt"]);
  });
});
