// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/test/native.test.mjs
// Responsibility:   Source file bindings/node/test/native.test.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { join } from "node:path";
import { describe, it } from "node:test";
import { fakeServe, internal, nurt, packageDirectory, withClient } from "./support.mjs";

const require = createRequire(import.meta.url);
const { Nurt, NurtError, NurtLanguageError, Response } = nurt;
const { locateAddon, nativeFailure } = internal("native");

const addonPath = join(packageDirectory, "test", "fixtures", "fake-addon.cjs");
const { observed } = require(addonPath);
const LIBRARY = { backend: "library", library: addonPath };
const CONCURRENT_CALLS = 100;

describe("library backend", () => {
  it("returns the response of a program", async () => {
    await withClient(LIBRARY, async (client) => {
      const response = await client.run("echo hello");
      assert.ok(response instanceof Response);
      assert.equal(response.ok, true);
      assert.equal(response.text, "hello");
    });
  });

  it("opens the engine with the session of the client", async () => {
    observed.engines.length = 0;
    await withClient({ ...LIBRARY, session: "work" }, async (client) => {
      await client.info();
    });
    assert.deepEqual(observed.engines, [{ session: "work" }]);
  });

  it("delivers the events of an execution", async () => {
    await withClient(LIBRARY, async (client) => {
      const seen = /** @type {string[]} */ ([]);
      const execution = client.start("echo done", { onEvent: (event) => seen.push(event.t) });
      const iterated = /** @type {string[]} */ ([]);
      for await (const event of execution) {
        iterated.push(event.t);
      }
      await execution.wait();
      assert.deepEqual(seen, ["exec.started", "job.output", "exec.finished"]);
      assert.deepEqual(iterated, seen);
    });
  });

  it("emits every event on the client", async () => {
    await withClient(LIBRARY, async (client) => {
      const seen = /** @type {string[]} */ ([]);
      client.on("event", (/** @type {any} */ event) => seen.push(event.t));
      await client.run("echo done");
      assert.deepEqual(seen, ["exec.started", "job.output", "exec.finished"]);
    });
  });

  it("cancels the execution when the signal is aborted", async () => {
    await withClient(LIBRARY, async (client) => {
      const before = observed.cancelled;
      const controller = new AbortController();
      const pending = client.run("slow", { signal: controller.signal });
      setTimeout(() => {
        controller.abort();
      }, 10);
      await assert.rejects(pending, { name: "NurtError", status: "NURT_E_CANCELLED" });
      assert.equal(observed.cancelled, before + 1);
    });
  });

  it("cancels an execution through its handle", async () => {
    await withClient(LIBRARY, async (client) => {
      const execution = client.start("slow");
      setTimeout(() => {
        execution.cancel();
      }, 10);
      const response = await execution.wait();
      assert.equal(response.ok, false);
      assert.equal(response.error.code, "E_CANCELLED");
    });
  });

  it("fails the calls in progress when the client is closed", async () => {
    const client = new Nurt(LIBRARY);
    const pending = client.run("slow");
    await new Promise((resolve) => setTimeout(resolve, 10));
    await client.close();
    await assert.rejects(pending, { status: "NURT_E_CANCELLED" });
    await assert.rejects(client.run("echo late"), { status: "NURT_E_ARG" });
  });

  it("maps the failures of the library to call failures", async () => {
    await withClient(LIBRARY, async (client) => {
      await assert.rejects(client.run("bad"), { name: "NurtError", status: "NURT_E_ARG", message: "the request is not valid" });
      await assert.rejects(client.run("broken"), { status: "NURT_E_INTERNAL", message: "the engine failed" });
      await assert.rejects(client.help("core"), { status: "NURT_E_IO" });
    });
  });

  it("returns the results of the other operations", async () => {
    await withClient(LIBRARY, async (client) => {
      assert.deepEqual(await client.parse("echo a", "b"), { operation: "parse", params: { program: "echo a", variant: "b" } });
      assert.deepEqual(await client.schema("report"), { operation: "schema", params: { name: "report" } });
      assert.equal(await client.describe({ layer: "group", name: "git", language: "pl" }), "git pl");
    });
  });

  it("returns the language error of parse as a failed report, as the server does", async () => {
    await withClient(LIBRARY, async (client) => {
      const result = await client.parse("bad");
      assert.equal(result.report.status, "failed");
      assert.equal(result.report.kind, "parse");
      assert.equal(result.report.error.code, "E_SYNTAX");
    });
  });

  it("throws the language error of a query", async () => {
    await withClient(LIBRARY, async (client) => {
      await assert.rejects(client.describe({ name: "missing" }), (/** @type {any} */ error) => {
        assert.ok(error instanceof NurtLanguageError);
        assert.equal(error.code, "E_NOT_FOUND");
        assert.equal(error.response.report.kind, "describe");
        return true;
      });
      await assert.rejects(client.describe({ name: "unknown" }), (/** @type {any} */ error) => {
        assert.ok(error instanceof NurtError);
        assert.equal(error.status, "NURT_E_ARG");
        assert.match(error.message, /^E_UNKNOWN_VALUE: unknown layer$/);
        assert.equal(error.detail.code, "E_UNKNOWN_VALUE");
        return true;
      });
    });
  });

  it("applies the language of the client to requests that name none", async () => {
    observed.requests.length = 0;
    await withClient({ ...LIBRARY, language: "pl" }, async (client) => {
      await client.run("echo one");
      await client.run("echo two", { language: "en" });
      await client.check("echo three");
      await client.help("core").catch(() => undefined);
      await client.describe({ layer: "group" });
    });
    const languages = observed.requests.map((request) => request.params.options?.language ?? request.params.language);
    assert.deepEqual(languages, ["pl", "en", "pl", "pl", "pl"]);
  });

  it("returns a description that is not text as its JSON text", async () => {
    await withClient(LIBRARY, async (client) => {
      const text = await client.describe({ name: "structured" });
      assert.deepEqual(JSON.parse(text), { layer: "core", commands: [] });
    });
  });

  it("uses the language en when the client names none", async () => {
    observed.requests.length = 0;
    await withClient(LIBRARY, async (client) => {
      await client.describe({ layer: "group" });
      await client.run("echo one");
    });
    const languages = observed.requests.map((request) => request.params.options?.language ?? request.params.language);
    assert.deepEqual(languages, ["en", "en"]);
  });

  it("rejects configuration overrides", async () => {
    await withClient({ ...LIBRARY, config: { journal: { max_ops: 5 } } }, async (client) => {
      await assert.rejects(client.info(), { status: "NURT_E_ARG" });
    });
  });

  it("runs calls concurrently", async () => {
    await withClient(LIBRARY, async (client) => {
      const responses = await Promise.all(Array.from({ length: CONCURRENT_CALLS }, (_, index) => client.run(`echo ${index}`)));
      assert.deepEqual(
        responses.map((response) => response.text),
        Array.from({ length: CONCURRENT_CALLS }, (_, index) => String(index)),
      );
    });
  });
});

describe("backend selection", () => {
  it("fails without a library when the library backend is required", async () => {
    await withClient({ backend: "library", library: join(packageDirectory, "missing.node"), env: { NURT_LIBRARY: "" } }, async (client) => {
      await assert.rejects(client.info(), (/** @type {any} */ error) => {
        assert.equal(error.status, "NURT_E_IO");
        assert.match(error.message, /missing\.node/);
        return true;
      });
    });
  });

  it("fails when the file is not a library", async () => {
    await withClient({ backend: "library", library: join(packageDirectory, "package.json") }, async (client) => {
      await assert.rejects(client.info(), { name: "NurtError" });
    });
  });

  it("uses the library in the automatic mode when one is found", async () => {
    await withClient({ library: addonPath, executable: "/nonexistent/nurt" }, async (client) => {
      assert.equal((await client.run("echo auto")).text, "auto");
    });
  });

  it("uses the process in the automatic mode when no library is found", async () => {
    await withClient({ ...fakeServe(), env: { NURT_LIBRARY: "" } }, async (client) => {
      assert.equal((await client.run("echo process")).text, "process");
    });
  });

  it("never loads a library for the process backend", async () => {
    await withClient({ ...fakeServe(), backend: "process", library: addonPath }, async (client) => {
      assert.equal((await client.run("echo process")).text, "process");
    });
  });

  it("falls back to the process when the library cannot be loaded in the automatic mode", async () => {
    await withClient({ ...fakeServe(), library: join(packageDirectory, "package.json") }, async (client) => {
      assert.equal((await client.run("echo process")).text, "process");
    });
  });
});

describe("locateAddon", () => {
  it("prefers the option to the variable", () => {
    const located = locateAddon({ library: addonPath, env: { NURT_LIBRARY: "/elsewhere/nurt.node" } });
    assert.equal(located.path, addonPath);
  });

  it("uses the variable when there is no option", () => {
    assert.equal(locateAddon({ env: { NURT_LIBRARY: addonPath } }).path, addonPath);
  });

  it("does not fall back after an explicit path that names no file", () => {
    const located = locateAddon({ library: "/nonexistent/nurt.node", env: {}, resolvePackage: () => packageDirectory });
    assert.equal(located.path, undefined);
    assert.match(located.checked[0], /the library option names .*nurt\.node, which is not a file/);
  });

  it("finds the addon in the bin directory of the platform package", () => {
    const located = locateAddon({ env: {}, resolvePackage: () => join(packageDirectory, "test", "fixtures", "package") });
    assert.equal(located.path, join(packageDirectory, "test", "fixtures", "package", "bin", "nurt.node"));
  });

  it("lists the packages that were checked", () => {
    const located = locateAddon({ env: {}, resolvePackage: () => undefined });
    assert.equal(located.path, undefined);
    assert.ok(located.checked.every((entry) => entry.includes("is not installed") || entry.startsWith("no platform package")));
  });
});

describe("nativeFailure", () => {
  it("reads the status from the message", () => {
    const failure = nativeFailure(new Error("NURT_E_TIMEOUT: too slow"));
    assert.equal(failure.status, "NURT_E_TIMEOUT");
    assert.equal(failure.message, "too slow");
  });

  it("treats other exceptions as internal failures", () => {
    assert.equal(nativeFailure(new Error("boom")).status, "NURT_E_INTERNAL");
    assert.equal(nativeFailure("text").message, "text");
  });
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
