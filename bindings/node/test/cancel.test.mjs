// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/test/cancel.test.mjs
// Responsibility:   Source file bindings/node/test/cancel.test.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { delay, fakeServe, withClient } from "./support.mjs";

/** Delay of a program that outlives every test. */
const LONG_MS = 60000;
/** Time for a request to reach the server. */
const SETTLE_MS = 50;

function isCancelled(error) {
  assert.equal(error.name, "NurtError");
  assert.equal(error.status, "NURT_E_CANCELLED");
  return true;
}

describe("cancellation", () => {
  it("rejects a call whose signal is already aborted without sending it", async () => {
    await withClient(fakeServe(), async (client) => {
      const controller = new AbortController();
      controller.abort();
      await assert.rejects(client.run(`delay ${LONG_MS} x`, { signal: controller.signal }), isCancelled);
      const info = await client.info();
      assert.equal(info.max_observed_inflight, 0);
      assert.equal(info.cancel_requests, 0);
    });
  });

  it("sends cancel for the request when the signal is aborted", async () => {
    await withClient(fakeServe(), async (client) => {
      // Connect first: an abort that arrives while the server is starting
      // happens before the request is sent, so no cancel is sent either.
      await client.info();
      const controller = new AbortController();
      const call = client.run(`delay ${LONG_MS} x`, { signal: controller.signal });
      await delay(SETTLE_MS);
      controller.abort();
      await assert.rejects(call, isCancelled);
      await delay(SETTLE_MS);
      const info = await client.info();
      assert.equal(info.max_observed_inflight, 1);
      assert.equal(info.cancel_requests, 1);
    });
  });

  it("cancels an execution and returns its response with E_CANCELLED from wait", async () => {
    await withClient(fakeServe(), async (client) => {
      await client.info();
      const execution = client.start(`delay ${LONG_MS} x`);
      await delay(SETTLE_MS);
      execution.cancel();
      const response = await execution.wait();
      assert.equal(response.ok, false);
      assert.equal(response.error.code, "E_CANCELLED");
      assert.equal(execution.done, true);
      execution.cancel();
      const info = await client.info();
      assert.equal(info.cancel_requests, 1);
    });
  });

  it("cancels an execution through the signal of start", async () => {
    await withClient(fakeServe(), async (client) => {
      await client.info();
      const controller = new AbortController();
      const execution = client.start(`delay ${LONG_MS} x`, { signal: controller.signal });
      await delay(SETTLE_MS);
      controller.abort();
      assert.equal((await execution.wait()).error.code, "E_CANCELLED");
    });
  });

  it("cancels an execution that is cancelled before its request is sent", async () => {
    await withClient(fakeServe(), async (client) => {
      const execution = client.start(`delay ${LONG_MS} x`);
      execution.cancel();
      assert.equal((await execution.wait()).error.code, "E_CANCELLED");
    });
  });

  it("throws the language error of a cancelled execution started with raiseOnError", async () => {
    await withClient(fakeServe(), async (client) => {
      await client.info();
      const execution = client.start(`delay ${LONG_MS} x`, { raiseOnError: true });
      await delay(SETTLE_MS);
      execution.cancel();
      await assert.rejects(execution.wait(), { name: "NurtLanguageError", code: "E_CANCELLED" });
    });
  });

  it("removes a queued call that is cancelled before it is sent", async () => {
    await withClient(fakeServe("--max-inflight", "1"), async (client) => {
      const first = client.run("delay 100 first");
      const controller = new AbortController();
      const queued = client.run("echo queued", { signal: controller.signal });
      controller.abort();
      await assert.rejects(queued, isCancelled);
      assert.equal((await first).text, "first");
      const info = await client.info();
      assert.equal(info.cancel_requests, 0);
    });
  });

  it("times out a wait while the execution continues", async () => {
    await withClient(fakeServe(), async (client) => {
      const execution = client.start("delay 200 late");
      await assert.rejects(execution.wait(10), { name: "NurtError", status: "NURT_E_TIMEOUT" });
      assert.equal(execution.done, false);
      assert.equal((await execution.wait()).text, "late");
    });
  });
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
