// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Runs the process backend against a built `nurt serve`.
//
//   NURT_BIN=<path of the nurt executable> node test/serve-smoke.mjs
//
// The check is not part of `npm test`, because the executable is a build
// product of the Rust workspace.

import assert from "node:assert/strict";
import { nurt } from "./support.mjs";

const { Nurt } = nurt;
const CONCURRENT_RUNS = 50;

const client = new Nurt({ backend: "process", executable: process.env.NURT_BIN });
try {
  const info = await client.info();
  assert.equal(typeof info.version, "string");

  const schema = await client.schema("request");
  assert.equal(typeof schema, "object");

  const events = [];
  const execution = client.start("find /", { onEvent: (event) => events.push(event) });
  const response = await execution.wait();
  assert.equal(typeof response.report.status, "string");
  assert.equal(events[0]?.t, "exec.started");
  assert.equal(events.at(-1)?.t, "exec.finished");
  assert.equal(typeof events[0].time, "string");

  const responses = await Promise.all(Array.from({ length: CONCURRENT_RUNS }, () => client.run("find /")));
  assert.equal(responses.length, CONCURRENT_RUNS);
} finally {
  await client.close();
}
