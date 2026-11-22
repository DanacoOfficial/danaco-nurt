// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Runs the language-neutral cases of conformance/bindings/ against a server.
//
// The server is the conformance test server when NURT_CONFORMANCE_EXECUTABLE
// names the nurt-conformance program (run as "fake-serve --cases <dir>"), or
// the nurt executable named by NURT_EXECUTABLE. Without either, or without
// cases, the test is skipped.
//
// A case file holds one object or an array of objects:
//   { "id": "...", "call": { "method": "run", "args": { ... } },
//     "expect": { "ok": true, "fields": { "report.status": "ok" } } }
// "expect" may instead give { "error": { "status": "NURT_E_ARG" } } for a
// call failure or { "language_error": "E_SYNTAX" } for a language error.

import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, it } from "node:test";
import { nurt, packageDirectory } from "./support.mjs";

const casesDirectory = join(packageDirectory, "..", "..", "conformance", "bindings");

function serverOptions() {
  const conformance = process.env.NURT_CONFORMANCE_EXECUTABLE;
  if (conformance !== undefined && conformance !== "") {
    return { executable: conformance, args: ["fake-serve", "--cases", casesDirectory] };
  }
  const executable = process.env.NURT_EXECUTABLE;
  return executable !== undefined && executable !== "" ? { executable } : undefined;
}

function loadCases() {
  if (!existsSync(casesDirectory)) {
    return [];
  }
  return readdirSync(casesDirectory)
    .filter((name) => name.endsWith(".json"))
    .sort()
    .flatMap((name) => {
      const content = JSON.parse(readFileSync(join(casesDirectory, name), "utf8"));
      return (Array.isArray(content) ? content : [content]).map((entry) => ({ file: name, ...entry }));
    });
}

/** Calls the client method that corresponds to a neutral method name. */
function invoke(client, method, args = {}) {
  const { options = {}, ...input } = args;
  switch (method) {
    case "run":
      return client.run(input, options);
    case "check":
      return client.check(input, options);
    case "parse":
      return client.parse(args.program, args.variant);
    case "print":
      return client.print(args.ast, args.variant);
    case "correct":
      return client.correct(args.program, args.variant);
    case "page":
      return client.page(args.cursor, args.limit);
    case "describe":
      return client.describe(args);
    case "help":
      return client.help(args.topic);
    case "catalog":
      return client.catalog(args.what);
    case "schema":
      return client.schema(args.which);
    case "history":
      return client.history(args.query);
    case "space":
      return client.space(args.volume);
    case "info":
      return client.info();
    default:
      throw new Error(`unknown method ${method}`);
  }
}

function valueAt(value, path) {
  return path.split(".").reduce((current, key) => (current === undefined || current === null ? undefined : current[key]), value);
}

async function check(client, testCase) {
  const { method, args } = testCase.call;
  const expect = testCase.expect ?? {};
  if (expect.error !== undefined) {
    await assert.rejects(invoke(client, method, args), (/** @type {any} */ error) => {
      assert.equal(error.name, "NurtError");
      assert.equal(error.status, expect.error.status);
      return true;
    });
    return;
  }
  const result = await invoke(client, method, args);
  const data = result instanceof nurt.Response ? result.raw : result;
  if (expect.ok !== undefined) {
    assert.equal(result.ok, expect.ok);
  }
  if (expect.language_error !== undefined) {
    assert.equal(result.error?.code, expect.language_error);
  }
  for (const [path, expected] of Object.entries(expect.fields ?? {})) {
    assert.deepEqual(valueAt(data, path), expected, path);
  }
}

const options = serverOptions();
const cases = loadCases();
const skip = options === undefined ? "no server: set NURT_CONFORMANCE_EXECUTABLE or NURT_EXECUTABLE" : cases.length === 0 ? "no cases" : false;

describe("conformance cases of the bindings", { skip }, () => {
  for (const testCase of cases) {
    it(`${testCase.file}: ${testCase.id}`, async () => {
      const client = new nurt.Nurt(options);
      try {
        await check(client, testCase);
      } finally {
        await client.close();
      }
    });
  }
});
