// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Runs the language-neutral cases of conformance/bindings/ against a server.
//
// The cases are in the format nurt-binding-cases/1. Each step calls the API of
// the client, and the outcome of the call is compared with the expectation of
// the step through JSON pointers into the result view of the call.
//
// The server is the conformance test server when NURT_CONFORMANCE_EXECUTABLE
// names the nurt-conformance program (started as "serve --stdio" with the
// variables NURT_CONFORMANCE_CASES and NURT_CONFORMANCE_CASE), or the nurt
// executable named by NURT_EXECUTABLE. Without either, or without cases, the
// test is skipped.

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, it } from "node:test";
import { isDeepStrictEqual } from "node:util";
import { nurt, packageDirectory } from "./support.mjs";

const CASES_FORMAT = "nurt-binding-cases/1";
const casesDirectory = join(packageDirectory, "..", "..", "conformance", "bindings");
const casesRoot = dirname(casesDirectory);
const STEP_TIMEOUT_MS = 30000;
const OPTIONAL_PREFIX = "optional:";
const PLACEHOLDER = /\$\{(base|step(\d+)(\/[^}]*))\}/;

/** Cases whose expectation this client does not meet, with the reason. */
const KNOWN_DIFFERENCES = new Map([
  [
    "start.cancel",
    "Execution.wait rejects with NURT_E_CANCELLED after cancel, whereas the case expects a response with the error E_CANCELLED",
  ],
]);

function nonEmpty(value) {
  return value === undefined || value === "" ? undefined : value;
}

/** Returns the options of a client for the server of a case, or undefined without a server. */
function serverOptions(id, stateDir) {
  const environment = { NURT_STATE_DIR: stateDir };
  const conformance = nonEmpty(process.env.NURT_CONFORMANCE_EXECUTABLE);
  if (conformance !== undefined) {
    return {
      backend: "process",
      executable: conformance,
      args: ["serve", "--stdio"],
      env: { ...environment, NURT_CONFORMANCE_CASES: casesRoot, NURT_CONFORMANCE_CASE: id },
    };
  }
  const executable = nonEmpty(process.env.NURT_EXECUTABLE);
  return executable === undefined ? undefined : { backend: "process", executable, env: environment };
}

/** Reads the case files of a directory; a file in another format is an error. */
function loadCases(directory) {
  let names;
  try {
    names = readdirSync(directory);
  } catch (error) {
    if (error.code === "ENOENT") {
      return [];
    }
    throw error;
  }
  return names
    .filter((name) => name.endsWith(".json"))
    .sort()
    .flatMap((name) => {
      const content = JSON.parse(readFileSync(join(directory, name), "utf8"));
      if (content === null || typeof content !== "object" || content.format !== CASES_FORMAT || !Array.isArray(content.cases)) {
        throw new Error(`${name} is not in the format ${CASES_FORMAT}`);
      }
      return content.cases.map((entry) => ({ file: name, ...entry }));
    });
}

/** Resolves a JSON pointer; the empty pointer addresses the document. */
function pointerGet(document, pointer) {
  let target = document;
  if (pointer === "") {
    return { found: true, value: target };
  }
  for (const raw of pointer.split("/").slice(1)) {
    const part = raw.replaceAll("~1", "/").replaceAll("~0", "~");
    if (Array.isArray(target)) {
      if (!/^(0|[1-9]\d*)$/.test(part) || Number(part) >= target.length) {
        return { found: false };
      }
      target = target[Number(part)];
    } else if (target !== null && typeof target === "object" && Object.hasOwn(target, part)) {
      target = target[part];
    } else {
      return { found: false };
    }
  }
  return { found: true, value: target };
}

function resolvePlaceholder(match, context) {
  if (match[1] === "base") {
    if (context.base === undefined) {
      throw new Error("the placeholder base is used by a case without fs");
    }
    return context.base;
  }
  const number = Number(match[2]);
  const view = context.views[number - 1];
  if (view === undefined) {
    throw new Error(`step ${number} has not run`);
  }
  const found = pointerGet(view, match[3]);
  if (!found.found) {
    throw new Error(`the result of step ${number} has no ${match[3]}`);
  }
  return found.value;
}

/** Replaces the placeholders base and stepN/pointer in the arguments of a step. */
function substitute(value, context) {
  if (typeof value === "string") {
    const whole = PLACEHOLDER.exec(value);
    if (whole !== null && whole.index === 0 && whole[0].length === value.length) {
      return resolvePlaceholder(whole, context);
    }
    return value.replace(new RegExp(PLACEHOLDER.source, "g"), (text) => String(resolvePlaceholder(PLACEHOLDER.exec(text), context)));
  }
  if (Array.isArray(value)) {
    return value.map((item) => substitute(item, context));
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, substitute(item, context)]));
  }
  return value;
}

function materialize(entries, root) {
  for (const entry of entries) {
    const path = join(root, ...entry.path.split("/"));
    mkdirSync(dirname(path), { recursive: true });
    if (entry.type === "dir") {
      mkdirSync(path, { recursive: true });
    } else if (entry.type === "symlink") {
      symlinkSync(entry.target, path);
    } else {
      writeFileSync(path, entry.content_utf8 ?? "");
    }
  }
}

/** The view of a response: ok, report, text, error and next_cursor. */
function responseView(response) {
  const report = response.report;
  const view = { ok: response.ok, report };
  if (response.error !== undefined) {
    view.error = response.error;
  }
  if (response.nextCursor !== undefined) {
    view.next_cursor = response.nextCursor;
  }
  if (response.text !== undefined) {
    view.text = response.text;
  }
  return view;
}

/** Splits the arguments of run, start and check into the program input and the options. */
function requestParts(args) {
  const { options = {}, raise_on_error: raiseOnError, program, variant, ast, cursor, accept, ...rest } = args;
  const input = {};
  for (const [key, value] of Object.entries({ program, variant, ast, cursor, accept })) {
    if (value !== undefined) {
      input[key] = value;
    }
  }
  return { input, options: { ...rest, ...options, ...(raiseOnError === true ? { raiseOnError: true } : {}) } };
}

async function callClient(client, executions, number, step, args) {
  switch (step.call) {
    case "run": {
      const { input, options } = requestParts(args);
      return responseView(await client.run(input, options));
    }
    case "start": {
      const { input, options } = requestParts(args);
      const events = [];
      const execution = client.start(input, { ...options, onEvent: (event) => events.push(event) });
      executions.set(number, { execution, events });
      return {};
    }
    case "check": {
      const { input, options } = requestParts(args);
      return { value: await client.check(input, options) };
    }
    case "parse":
      return { value: await client.parse(args.program, args.variant) };
    case "print":
      return { value: await client.print(args.ast, args.variant) };
    case "correct":
      return { value: await client.correct(args.program, args.variant) };
    case "page":
      return responseView(await client.page(args.cursor, args.limit));
    case "describe":
      return { value: await client.describe({ layer: args.layer ?? "core", name: args.name, language: args.language }) };
    case "help":
      return { value: await client.help(args.topic) };
    case "catalog":
      return { value: await client.catalog(args.what) };
    case "schema":
      return { value: await client.schema(args.which) };
    case "history":
      return { value: await client.history(args.query) };
    case "space":
      return { value: await client.space(args.volume) };
    case "info":
      return { value: await client.info() };
    case "close":
      await client.close();
      return {};
    case "wait":
    case "cancel":
    case "events": {
      const started = executions.get(step.execution);
      if (started === undefined) {
        throw new Error(`step ${step.execution} has started no execution`);
      }
      if (step.call === "cancel") {
        started.execution.cancel();
        return {};
      }
      if (step.call === "wait") {
        return responseView(await started.execution.wait(args.timeout_ms ?? STEP_TIMEOUT_MS));
      }
      await started.execution.wait(STEP_TIMEOUT_MS).catch(() => undefined);
      return { value: started.events };
    }
    default:
      throw new Error(`unknown call ${step.call}`);
  }
}

/** Sorts the result of a call into ok, a language error or a call error. */
function classify(call, view, error) {
  if (error !== undefined) {
    if (error instanceof nurt.NurtLanguageError) {
      return { kind: "language_error", view: responseView(error.response), code: error.code };
    }
    const code = error?.detail?.code;
    if (error instanceof nurt.NurtError && typeof code === "string") {
      return { kind: "language_error", view: {}, code };
    }
    return { kind: "call_error", view: {}, status: error instanceof nurt.NurtError ? error.status : "NURT_E_INTERNAL" };
  }
  if (["run", "page", "wait"].includes(call) && view.ok === false) {
    return { kind: "language_error", view, code: view.error?.code };
  }
  for (const pointer of ["/value/error/code", "/value/report/error/code"]) {
    const found = pointerGet(view, pointer);
    if (found.found && typeof found.value === "string") {
      return { kind: "language_error", view, code: found.value };
    }
  }
  return { kind: "ok", view };
}

/** Returns the unmet expectations of a step. */
function unmetExpectations(expect, got) {
  const found = [];
  if (expect.outcome !== undefined && got.kind !== expect.outcome) {
    found.push(`outcome ${got.kind}, expected ${expect.outcome} (status ${got.status ?? ""}, code ${got.code ?? ""})`);
  }
  if (expect.status !== undefined && got.status !== expect.status) {
    found.push(`status ${got.status}, expected ${expect.status}`);
  }
  if (expect.code !== undefined && got.code !== expect.code) {
    found.push(`code ${got.code}, expected ${expect.code}`);
  }
  for (const [pointer, want] of Object.entries(expect.fields ?? {}).sort(([left], [right]) => (left < right ? -1 : 1))) {
    const actual = pointerGet(got.view, pointer);
    if (!actual.found) {
      found.push(`${pointer} is missing`);
    } else if (!isDeepStrictEqual(actual.value, want)) {
      found.push(`${pointer} is ${JSON.stringify(actual.value)}, expected ${JSON.stringify(want)}`);
    }
  }
  for (const pointer of expect.present ?? []) {
    if (!pointerGet(got.view, pointer).found) {
      found.push(`${pointer} is missing`);
    }
  }
  for (const pointer of expect.absent ?? []) {
    if (pointerGet(got.view, pointer).found) {
      found.push(`${pointer} is present`);
    }
  }
  return found;
}

/** Returns the reason a case does not apply to this client, or false. */
function skipReason(testCase) {
  const known = KNOWN_DIFFERENCES.get(testCase.id);
  if (known !== undefined) {
    return known;
  }
  if (Array.isArray(testCase.backends) && !testCase.backends.includes("process")) {
    return "the case is not for the process backend";
  }
  for (const condition of testCase.requires ?? []) {
    if ((condition === "os:windows") !== (process.platform === "win32") && condition.startsWith("os:")) {
      return `the case requires ${condition}`;
    }
  }
  return false;
}

/** Runs the steps of a case and returns the unmet expectations, or the reason to skip. */
async function runCase(testCase) {
  const stateDir = mkdtempSync(join(tmpdir(), "nurt-state-"));
  const hasFiles = Array.isArray(testCase.fs) && testCase.fs.length > 0;
  const base = hasFiles ? mkdtempSync(join(tmpdir(), "nurt-case-")) : undefined;
  const context = { base, views: [] };
  const executions = new Map();
  const problems = [];
  const client = new nurt.Nurt(serverOptions(testCase.id, stateDir));
  try {
    if (base !== undefined) {
      materialize(testCase.fs, base);
    }
    if (typeof testCase.class === "string" && testCase.class.startsWith(OPTIONAL_PREFIX)) {
      const feature = testCase.class.slice(OPTIONAL_PREFIX.length);
      const info = await client.info();
      if (!Array.isArray(info.features) || !info.features.includes(feature)) {
        return { problems, skipped: `the implementation does not declare the feature ${feature}` };
      }
    }
    for (const [index, step] of testCase.steps.entries()) {
      const number = index + 1;
      let got;
      try {
        const args = substitute(step.args ?? {}, context);
        let view;
        let failure;
        try {
          view = await callClient(client, executions, number, step, args);
        } catch (error) {
          failure = error;
        }
        got = classify(step.call, view, failure);
      } catch (error) {
        problems.push(`step ${number}: ${error.message}`);
        break;
      }
      context.views.push(got.view);
      for (const item of unmetExpectations(step.expect ?? {}, got)) {
        problems.push(`step ${number} (${step.call}): ${item}`);
      }
    }
  } finally {
    for (const { execution } of executions.values()) {
      execution.cancel();
    }
    await client.close();
    rmSync(stateDir, { recursive: true, force: true });
    if (base !== undefined) {
      rmSync(base, { recursive: true, force: true });
    }
  }
  return { problems, skipped: undefined };
}

describe("runner of the binding cases", () => {
  it("resolves JSON pointers to members and items", () => {
    const document = { value: [{ t: "exec.started" }], "a/b": { "~": 1 } };
    assert.deepEqual(pointerGet(document, "/value/0/t"), { found: true, value: "exec.started" });
    assert.deepEqual(pointerGet(document, "/a~1b/~0"), { found: true, value: 1 });
    assert.equal(pointerGet(document, "/value/1").found, false);
    assert.equal(pointerGet(document, "/value/-1").found, false);
    assert.equal(pointerGet(document, "/missing").found, false);
    assert.deepEqual(pointerGet(document, ""), { found: true, value: document });
  });

  it("takes placeholders from the base and from earlier steps", () => {
    const context = { base: "/home/user/case", views: [{ next_cursor: "c-1", limit: 2 }] };
    assert.deepEqual(
      substitute({ cursor: "${step1/next_cursor}", limit: "${step1/limit}", path: "${base}/src", list: ["${base}"] }, context),
      { cursor: "c-1", limit: 2, path: "/home/user/case/src", list: ["/home/user/case"] },
    );
    assert.throws(() => substitute("${step2/next_cursor}", context), /step 2 has not run/);
    assert.throws(() => substitute("${base}", { views: [] }), /without fs/);
  });

  it("moves the envelope fields and the options out of the arguments", () => {
    const { input, options } = requestParts({
      program: "count",
      cursor: "c",
      limit: 1,
      options: { timeout: "1500ms", base: "/home/user" },
      raise_on_error: true,
    });
    assert.deepEqual(input, { program: "count", cursor: "c" });
    assert.deepEqual(options, { limit: 1, timeout: "1500ms", base: "/home/user", raiseOnError: true });
  });

  it("sorts outcomes and reports every unmet expectation", () => {
    const language = classify("run", { ok: false, error: { code: "E_SYNTAX" } }, undefined);
    assert.equal(language.kind, "language_error");
    assert.equal(language.code, "E_SYNTAX");
    const failure = classify("info", undefined, new nurt.NurtError("NURT_E_ARG", "closed"));
    assert.deepEqual([failure.kind, failure.status], ["call_error", "NURT_E_ARG"]);
    const library = classify("parse", undefined, new nurt.NurtError("NURT_E_ARG", "E_LEXICAL: bad", { detail: { code: "E_LEXICAL" } }));
    assert.deepEqual([library.kind, library.code], ["language_error", "E_LEXICAL"]);
    const embedded = classify("parse", { value: { report: { error: { code: "E_LEXICAL" } } } }, undefined);
    assert.equal(embedded.code, "E_LEXICAL");
    const problems = unmetExpectations(
      { outcome: "ok", fields: { "/report/kind": "find" }, present: ["/error"], absent: ["/report"] },
      { kind: "ok", view: { report: { kind: "count" } } },
    );
    assert.equal(problems.length, 3);
  });

  it("rejects a case file in another format", () => {
    const directory = mkdtempSync(join(tmpdir(), "nurt-cases-"));
    try {
      writeFileSync(join(directory, "old.json"), JSON.stringify({ id: "x", call: { method: "run" } }));
      assert.throws(() => loadCases(directory), /not in the format nurt-binding-cases\/1/);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it("reads the cases of the repository", () => {
    for (const testCase of loadCases(casesDirectory)) {
      assert.equal(typeof testCase.id, "string");
      assert.ok(Array.isArray(testCase.steps) && testCase.steps.length > 0, testCase.id);
    }
  });
});

const available = nonEmpty(process.env.NURT_CONFORMANCE_EXECUTABLE) ?? nonEmpty(process.env.NURT_EXECUTABLE);
const cases = loadCases(casesDirectory);
const skip = available === undefined ? "no server: set NURT_CONFORMANCE_EXECUTABLE or NURT_EXECUTABLE" : cases.length === 0 ? "no cases" : false;

describe("conformance cases of the bindings", { skip }, () => {
  for (const testCase of cases) {
    it(`${testCase.file}: ${testCase.id}`, { skip: skipReason(testCase) }, async (context) => {
      const result = await runCase(testCase);
      if (result.skipped !== undefined) {
        context.skip(result.skipped);
        return;
      }
      assert.deepEqual(result.problems, []);
    });
  }
});
