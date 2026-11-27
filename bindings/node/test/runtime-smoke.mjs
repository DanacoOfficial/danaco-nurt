// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/test/runtime-smoke.mjs
// Responsibility:   Source file bindings/node/test/runtime-smoke.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// Checks the package in Bun and Deno, which do not run the node:test suite:
// a run, an execution with events, cancellation and close against the test
// server. Exits with status 0 on success.
//
//   bun test/runtime-smoke.mjs
//   deno run --allow-read --allow-env --allow-run test/runtime-smoke.mjs

import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Nurt, NurtError } from "../esm/index.mjs";

const fakeServe = join(dirname(fileURLToPath(import.meta.url)), "fixtures", "fake-serve.mjs");
const args = typeof globalThis.Deno === "undefined" ? [fakeServe] : ["run", fakeServe];

function expect(condition, message) {
  if (!condition) {
    throw new Error(`runtime smoke test failed: ${message}`);
  }
}

const client = new Nurt({ executable: process.execPath, args });
try {
  const response = await client.run("echo smoke");
  expect(response.ok && response.text === "smoke", "run returns the rendered text");

  const types = [];
  for await (const event of client.start("events 2")) {
    types.push(event.t);
  }
  expect(types.join(",") === "job.output,job.output,exec.finished", `events ${types.join(",")}`);

  const controller = new AbortController();
  const slow = client.run("delay 60000 x", { signal: controller.signal });
  controller.abort();
  const cancelled = await slow.then(
    () => false,
    (error) => error instanceof NurtError && error.status === "NURT_E_CANCELLED",
  );
  expect(cancelled, "an aborted call fails with NURT_E_CANCELLED");
} finally {
  await client.close();
}
process.stdout.write("runtime smoke test passed\n");

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
