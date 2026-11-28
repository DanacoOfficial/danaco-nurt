// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           integrations/vscode/test/suite/extension.test.ts
// Responsibility:   Source file integrations/vscode/test/suite/extension.test.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import * as assert from "assert";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";

const EXTENSION_ID = "danaco.nurt";

async function configure(key: string, value: unknown): Promise<void> {
  await vscode.workspace
    .getConfiguration("nurt")
    .update(key, value, vscode.ConfigurationTarget.Global);
}

function fixture(): vscode.Uri {
  const folder = vscode.workspace.workspaceFolders?.[0];
  assert.ok(folder !== undefined, "the test workspace is open");
  return vscode.Uri.file(path.join(folder.uri.fsPath, "sample.nurt"));
}

suite("extension", () => {
  suiteSetup(async () => {
    await configure("lsp.enabled", false);
    const extension = vscode.extensions.getExtension(EXTENSION_ID);
    assert.ok(extension !== undefined, "the extension is installed");
    await extension.activate();
  });

  suiteTeardown(async () => {
    await configure("lsp.enabled", undefined);
    await configure("path", undefined);
  });

  test("activates for a nurt file", async () => {
    const document = await vscode.workspace.openTextDocument(fixture());
    assert.strictEqual(document.languageId, "nurt");
    assert.strictEqual(vscode.extensions.getExtension(EXTENSION_ID)?.isActive, true);
  });

  test("registers the commands", async () => {
    const commands = await vscode.commands.getCommands(true);
    for (const command of ["nurt.runFile", "nurt.formatFile", "nurt.restartServer"]) {
      assert.ok(commands.includes(command), `${command} is registered`);
    }
  });

  test("defaults nurt.path to the bare executable name", () => {
    const inspected = vscode.workspace.getConfiguration("nurt").inspect<string>("path");
    assert.strictEqual(inspected?.defaultValue, "nurt");
  });

  test("restarts without a running server when the server is disabled", async () => {
    await vscode.commands.executeCommand("nurt.restartServer");
  });

  test("formats a file with the executable from nurt.path", async function () {
    if (process.platform === "win32") {
      this.skip();
    }
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "nurt-ext-"));
    try {
      const stub = path.join(directory, "nurt");
      fs.writeFileSync(stub, '#!/bin/sh\nif [ "$1" = fmt ]; then printf \'list "."\\n\'; fi\n', {
        mode: 0o755,
      });
      const file = path.join(directory, "format.nurt");
      fs.writeFileSync(file, 'list    "."\n');
      await configure("path", stub);
      const document = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
      await vscode.window.showTextDocument(document);
      await vscode.commands.executeCommand("nurt.formatFile");
      assert.strictEqual(document.getText(), 'list "."\n');
    } finally {
      fs.rmSync(directory, { recursive: true, force: true });
    }
  });
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
