// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           integrations/vscode/test/unit/manifest.test.ts
// Responsibility:   Source file integrations/vscode/test/unit/manifest.test.ts.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import * as assert from "assert";
import * as fs from "fs";
import * as path from "path";

const root = path.resolve(__dirname, "..", "..", "..");

interface Manifest {
  publisher: string;
  name: string;
  license: string;
  dependencies: Record<string, string>;
  devDependencies: Record<string, string>;
  contributes: {
    languages: { id: string; extensions: string[]; configuration: string }[];
    grammars: { language: string; scopeName: string; path: string }[];
    commands: { command: string }[];
    configuration: { properties: Record<string, { default?: unknown }> };
  };
}

const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")) as Manifest;

suite("manifest", () => {
  test("identifies the extension as danaco.nurt", () => {
    assert.strictEqual(`${manifest.publisher}.${manifest.name}`, "danaco.nurt");
    assert.strictEqual(manifest.license, "Apache-2.0");
  });

  test("contributes the nurt language for .nurt files", () => {
    const language = manifest.contributes.languages.find((entry) => entry.id === "nurt");
    assert.ok(language !== undefined);
    assert.deepStrictEqual(language.extensions, [".nurt"]);
    assert.ok(fs.existsSync(path.join(root, language.configuration)));
  });

  test("binds one grammar to the nurt language", () => {
    assert.strictEqual(manifest.contributes.grammars.length, 1);
    assert.strictEqual(manifest.contributes.grammars[0].language, "nurt");
    assert.strictEqual(manifest.contributes.grammars[0].path, "./syntaxes/nurt.tmLanguage.json");
  });

  test("contributes the three commands", () => {
    const commands = manifest.contributes.commands.map((entry) => entry.command).sort();
    assert.deepStrictEqual(commands, ["nurt.formatFile", "nurt.restartServer", "nurt.runFile"]);
  });

  test("defines nurt.path with the bare executable name as default", () => {
    assert.strictEqual(manifest.contributes.configuration.properties["nurt.path"].default, "nurt");
  });

  test("uses vscode-languageclient and no telemetry library", () => {
    assert.ok("vscode-languageclient" in manifest.dependencies);
    const all = Object.keys({ ...manifest.dependencies, ...manifest.devDependencies });
    const telemetry = all.filter((name) => /telemetry|applicationinsights|sentry|segment/i.test(name));
    assert.deepStrictEqual(telemetry, []);
  });

  test("pairs only the double quote because texts are written in double quotes", () => {
    const language = manifest.contributes.languages.find((entry) => entry.id === "nurt");
    assert.ok(language !== undefined);
    const configuration = JSON.parse(
      fs.readFileSync(path.join(root, language.configuration), "utf8"),
    ) as { autoClosingPairs: { open: string }[]; surroundingPairs: string[][] };
    assert.deepStrictEqual(
      configuration.autoClosingPairs.map((pair) => pair.open).filter((open) => /['"]/.test(open)),
      ['"'],
    );
    assert.deepStrictEqual(
      configuration.surroundingPairs.filter((pair) => /['"]/.test(pair[0])),
      [['"', '"']],
    );
  });

  test("defines no telemetry setting", () => {
    const keys = Object.keys(manifest.contributes.configuration.properties);
    assert.deepStrictEqual(
      keys.filter((key) => /telemetry/i.test(key)),
      [],
    );
  });
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
