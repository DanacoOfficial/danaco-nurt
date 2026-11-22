// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Runs the library backend against the addon built from crates/nurt-node.
//
//   NURT_LIBRARY=<path of nurt.node> node test/addon-smoke.mjs
//
// The check is part of the CI workflow and is not run by `npm test`, because
// the addon is a build product of the Rust workspace.

import assert from "node:assert/strict";
import { nurt } from "./support.mjs";

const { Nurt } = nurt;
const CONCURRENT_RUNS = 50;

const client = new Nurt({ backend: "library" });
try {
  const info = await client.info();
  assert.equal(typeof info.version, "string");

  const schema = await client.schema("request");
  assert.equal(typeof schema, "object");

  const events = [];
  const execution = client.start("find /", { onEvent: (event) => events.push(event.t) });
  const response = await execution.wait();
  assert.equal(typeof response.report.status, "string");
  assert.equal(events[0], "exec.started");
  assert.equal(events.at(-1), "exec.finished");

  const responses = await Promise.all(Array.from({ length: CONCURRENT_RUNS }, () => client.run("find /")));
  assert.equal(responses.length, CONCURRENT_RUNS);
} finally {
  await client.close();
}
