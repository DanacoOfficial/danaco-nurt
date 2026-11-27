// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Runs every method of the process backend against the real engine
// `nurt serve --stdio`. The tests are skipped unless NURT_TEST_EXECUTABLE names
// the nurt executable.

import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, it } from "node:test";
import { nurt, withClient } from "./support.mjs";

const executable = process.env.NURT_TEST_EXECUTABLE;
const skip = executable === undefined || executable === "" ? "no engine: set NURT_TEST_EXECUTABLE to the nurt executable" : false;

/** Options of a client of the real engine with its own state directory. */
function engine() {
  return {
    backend: "process",
    executable,
    env: { ...process.env, NURT_STATE_DIR: mkdtempSync(join(tmpdir(), "nurt-engine-state-")) },
  };
}

/** A directory with the given number of files, as the program text that lists it. */
function listing(files) {
  const directory = mkdtempSync(join(tmpdir(), "nurt-engine-files-"));
  for (let index = 0; index < files; index += 1) {
    writeFileSync(join(directory, `file-${index}.txt`), "text\n");
  }
  return `list "${directory.replaceAll("\\", "/")}"`;
}

describe("the real engine", { skip }, () => {
  it("reports its formats in info", async () => {
    await withClient(engine(), async (client) => {
      const info = await client.info();
      assert.equal(info.formats.ast, "1.1");
      assert.equal(info.formats.bound, "1.1");
      assert.equal(typeof info.version, "string");
    });
  });

  it("answers catalog selections", async () => {
    await withClient(engine(), async (client) => {
      assert.equal((await client.catalog()).format, "nurt-catalog-result/1");
      assert.ok((await client.catalog({ group: "core" })).commands.length > 0);
      assert.equal((await client.catalog({ command: "list" })).command.name, "list");
    });
  });

  it("serves a schema and rejects an unknown one", async () => {
    await withClient(engine(), async (client) => {
      assert.equal((await client.schema("ast")).title, "nurt-ast/1");
      await assert.rejects(client.schema("unknown"), (error) => error.status === "NURT_E_ARG");
    });
  });

  it("returns the text of describe and of help", async () => {
    await withClient(engine(), async (client) => {
      const description = await client.describe();
      assert.ok(description.includes("PROGRAM"));
      assert.ok(!description.startsWith("{"));
      await assert.rejects(client.describe({ name: "no-such-command" }), (error) => error instanceof nurt.NurtLanguageError);
      assert.ok((await client.help()).startsWith("GROUP"));
      assert.ok((await client.help("list")).length > 0);
    });
  });

  it("runs a program and continues a shortened result", async () => {
    await withClient(engine(), async (client) => {
      const response = await client.run(listing(5), { limit: 2 });
      assert.equal(response.ok, true);
      assert.equal(response.report.kind, "list");
      assert.equal(response.raw.report.continuation.remaining, 3);
      assert.equal(typeof response.nextCursor, "string");
      const next = await client.run("next");
      assert.equal(next.ok, true);
      assert.equal(next.report.kind, "next");
      assert.equal((await client.page(response.nextCursor)).ok, true);
    });
  });

  it("returns a language error in the response", async () => {
    await withClient(engine(), async (client) => {
      const response = await client.run('lst "/tmp"');
      assert.equal(response.ok, false);
      assert.equal(response.error.code, "E_UNKNOWN_COMMAND");
      await assert.rejects(client.run('lst "/tmp"', { raiseOnError: true }), (error) => error instanceof nurt.NurtLanguageError);
      const page = await client.page("malformed");
      assert.equal(page.error.code, "E_ARG");
    });
  });

  it("delivers the events of an execution", async () => {
    await withClient(engine(), async (client) => {
      const events = [];
      const response = await client.start(listing(1), { onEvent: (event) => events.push(event) }).wait();
      assert.equal(response.ok, true);
      assert.equal(events[0].t, "exec.started");
      assert.equal(events.at(-1).t, "exec.finished");
    });
  });

  it("checks, parses, prints and corrects", async () => {
    await withClient(engine(), async (client) => {
      const checked = await client.check('list "/tmp"');
      assert.equal(typeof checked.bound, "object");
      const parsed = await client.parse('list "/tmp"');
      assert.equal(parsed.ast.nurt, "1.1");
      assert.ok((await client.print(parsed.ast)).startsWith("list"));
      const broken = await client.parse("list (");
      assert.equal(broken.report.error.code, "E_SYNTAX");
      assert.equal((await client.correct('list "/tmp"')).program, 'list "/tmp"');
      const quoted = await client.correct("find '/tmp'");
      assert.equal(quoted.program, 'find "/tmp"');
      assert.ok(quoted.corrections.length > 0);
    });
  });

  it("answers history and space", async () => {
    await withClient(engine(), async (client) => {
      assert.ok(Array.isArray((await client.history()).operations));
      assert.ok(Array.isArray((await client.history('{"limit":3}')).operations));
      assert.ok(Array.isArray((await client.space()).volumes));
    });
  });

  it("accepts the options privilege and asUser", async () => {
    await withClient(engine(), async (client) => {
      for (const privilege of ["account", "auto"]) {
        assert.equal((await client.run(listing(1), { privilege })).ok, true);
      }
      const response = await client.run(listing(1), { as_user: "root" });
      if (response.ok) {
        assert.equal(response.raw.report.as_user, "root");
      } else {
        assert.match(response.error.code, /^E_/);
      }
    });
  });
});
