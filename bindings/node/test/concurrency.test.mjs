// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { fakeServe, withClient } from "./support.mjs";

const CALLS = 100;
/** Spread of the response delays, so that responses arrive out of order. */
const DELAY_SPREAD_MS = 7;

describe("concurrent calls", () => {
  it("matches 100 parallel calls to their responses", async () => {
    await withClient(fakeServe(), async (client) => {
      const calls = Array.from({ length: CALLS }, (_, index) =>
        client.run(`delay ${(CALLS - index) % DELAY_SPREAD_MS} call-${index}`),
      );
      const responses = await Promise.all(calls);
      responses.forEach((response, index) => {
        assert.equal(response.text, `call-${index}`);
      });
    });
  });

  it("keeps the number of requests in progress within max_inflight", async () => {
    const limit = 4;
    await withClient(fakeServe("--max-inflight", String(limit)), async (client) => {
      const calls = Array.from({ length: CALLS }, (_, index) => client.run(`delay ${index % DELAY_SPREAD_MS} q-${index}`));
      const responses = await Promise.all(calls);
      assert.deepEqual(
        responses.map((response) => response.text),
        Array.from({ length: CALLS }, (_, index) => `q-${index}`),
      );
      const info = await client.info();
      assert.ok(info.max_observed_inflight <= limit, `observed ${info.max_observed_inflight} requests in progress`);
      assert.ok(info.max_observed_inflight > 1, "requests ran concurrently");
    });
  });

  it("starts one server for concurrent first calls", async () => {
    await withClient(fakeServe(), async (client) => {
      await Promise.all([client.run("delay 30 a"), client.run("delay 30 b")]);
      const info = await client.info();
      assert.equal(info.max_observed_inflight, 2);
    });
  });
});
