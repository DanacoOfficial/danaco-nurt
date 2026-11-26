// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, it } from "node:test";
import { fakeServe, internal, nurt, packageDirectory, withClient } from "./support.mjs";

const { Nurt, NurtError, NurtLanguageError, Response } = nurt;
const { statusOfRpcCode } = internal("connection");

describe("Nurt", () => {
  it("returns the response of a program", async () => {
    await withClient(fakeServe(), async (client) => {
      const response = await client.run("echo hello world");
      assert.ok(response instanceof Response);
      assert.equal(response.ok, true);
      assert.equal(response.text, "hello world");
      assert.equal(response.report.status, "ok");
      assert.equal(response.error, undefined);
      assert.equal(response.raw.nurt, "1.0");
    });
  });

  it("lists the report of every pipeline of a script", () => {
    const script = new Response({
      nurt: "1.0",
      report: { nurt: "1.0", status: "ok", kind: "remove" },
      reports: [
        { nurt: "1.0", status: "ok", kind: "find" },
        { nurt: "1.0", status: "ok", kind: "remove" },
      ],
    });
    assert.deepEqual(
      script.reports.map((report) => report.kind),
      ["find", "remove"],
    );
    const single = new Response({ nurt: "1.0", report: { nurt: "1.0", status: "ok", kind: "find" } });
    assert.deepEqual(single.reports, []);
  });

  it("returns a language error in the response", async () => {
    await withClient(fakeServe(), async (client) => {
      const response = await client.run("fail");
      assert.equal(response.ok, false);
      assert.equal(response.error.code, "E_SYNTAX");
      assert.deepEqual(response.error.at, { line: 1, column: 5 });
    });
  });

  it("throws NurtLanguageError when raiseOnError is set", async () => {
    await withClient(fakeServe(), async (client) => {
      await assert.rejects(client.run("fail", { raiseOnError: true }), (/** @type {any} */ error) => {
        assert.ok(error instanceof NurtLanguageError);
        assert.equal(error.code, "E_SYNTAX");
        assert.deepEqual(error.fix, ["echo text"]);
        assert.equal(error.help, "syntax");
        assert.equal(error.response.ok, false);
        return true;
      });
    });
  });

  it("sends the request forms and options of nurt-request/1", async () => {
    await withClient(fakeServe(), async (client) => {
      const checked = await client.check({ program: "echo a", variant: "b" }, { preview: true, signal: undefined });
      assert.equal(checked.program, "echo a");
      const page = await client.run("page");
      assert.equal(page.nextCursor, "cursor-2");
      const next = await client.page("cursor-2", 10);
      assert.equal(next.text, "page cursor-2 10");
    });
  });

  it("calls every method of the interface", async () => {
    await withClient(fakeServe(), async (client) => {
      assert.equal((await client.parse("echo a")).variant, "a");
      assert.equal(await client.print({ nurt: "1.0", program: { statements: [] } }, "c"), "print c");
      assert.equal((await client.correct("echo a")).program, "echo a");
      assert.equal((await client.page("cursor-1")).text, "page cursor-1 default");
      assert.equal(await client.describe(), "core en");
      assert.equal(await client.describe({ layer: "group", name: "git", language: "pl" }), "git pl");
      assert.equal(await client.help(), "index");
      assert.equal(await client.help("syntax"), "syntax");
      assert.deepEqual(await client.catalog("core"), { method: "catalog", params: { what: "core" } });
      assert.deepEqual(await client.schema("report"), { method: "schema", params: { name: "report" } });
      assert.deepEqual(await client.history(), { method: "history", params: {} });
      assert.deepEqual(await client.space("/"), { method: "space", params: { volume: "/" } });
    });
  });

  it("announces the client, language, session and configuration in initialize", async () => {
    const manifest = JSON.parse(readFileSync(join(packageDirectory, "package.json"), "utf8"));
    const options = { ...fakeServe(), language: "pl", session: "session-1", config: { syntax: { variant: "b" } } };
    await withClient(options, async (client) => {
      const info = await client.info();
      assert.deepEqual(info.client, { name: "nurt-node", version: manifest.version });
      assert.equal(info.session, "session-1");
      assert.deepEqual(info.config_overrides, { syntax: { variant: "b" } });
    });
  });

  it("describes in the language of the client when the call names none", async () => {
    await withClient({ ...fakeServe(), language: "pl" }, async (client) => {
      assert.equal(await client.describe(), "core pl");
      assert.equal(await client.describe({ language: "en" }), "core en");
    });
  });

  it("rejects a server of another protocol version with NURT_E_ABI", async () => {
    await withClient(fakeServe("--protocol", "nurt-rpc/2"), async (client) => {
      await assert.rejects(client.info(), (/** @type {any} */ error) => {
        assert.ok(error instanceof NurtError);
        assert.equal(error.status, "NURT_E_ABI");
        assert.match(error.message, /nurt-rpc\/2/);
        return true;
      });
    });
  });

  it("reports a missing executable with NURT_E_IO", async () => {
    await withClient({ executable: join(packageDirectory, "test", "missing", "nurt") }, async (client) => {
      await assert.rejects(client.run("echo a"), { name: "NurtError", status: "NURT_E_IO" });
    });
  });

  it("rejects an unknown backend", () => {
    assert.throws(() => new Nurt({ backend: /** @type {any} */ ("thread") }), { name: "NurtError", status: "NURT_E_ARG" });
  });

  it("rejects calls after close", async () => {
    const client = new Nurt(fakeServe());
    await client.run("echo a");
    await client.close();
    await assert.rejects(client.run("echo b"), { name: "NurtError", status: "NURT_E_ARG" });
    await client.close();
  });

  it("maps JSON-RPC errors to status names", async () => {
    assert.equal(statusOfRpcCode(-32001), "NURT_E_TIMEOUT");
    assert.equal(statusOfRpcCode(-32002), "NURT_E_CANCELLED");
    assert.equal(statusOfRpcCode(-32003), "NURT_E_ARG");
    assert.equal(statusOfRpcCode(-32004), "NURT_E_ABI");
    assert.equal(statusOfRpcCode(-32005), "NURT_E_IO");
    assert.equal(statusOfRpcCode(-32006), "NURT_E_INTERNAL");
    assert.equal(statusOfRpcCode(-32602), "NURT_E_ARG");
    assert.equal(statusOfRpcCode(-32601), "NURT_E_INTERNAL");
    assert.equal(statusOfRpcCode(-32700), "NURT_E_INTERNAL");
    assert.equal(statusOfRpcCode(-32050), "NURT_E_INTERNAL");
  });
});
