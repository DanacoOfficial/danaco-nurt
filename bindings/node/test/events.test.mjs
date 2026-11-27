// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/test/events.test.mjs
// Responsibility:   Source file bindings/node/test/events.test.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { fakeServe, nurt, withClient } from "./support.mjs";

const { Execution } = nurt;

const EVENT_COUNT = 3;
const EXPECTED_TYPES = ["job.output", "job.output", "job.output", "exec.finished"];

describe("events of an execution", () => {
  it("iterates the events with for await until the execution ends", async () => {
    await withClient(fakeServe(), async (client) => {
      const execution = client.start(`events ${EVENT_COUNT}`);
      assert.ok(execution instanceof Execution);
      const types = [];
      for await (const event of execution) {
        types.push(event.t);
      }
      assert.deepEqual(types, EXPECTED_TYPES);
      assert.equal((await execution.wait()).text, `events ${EVENT_COUNT}`);
    });
  });

  it("emits the events to listeners, onEvent and the client", async () => {
    await withClient(fakeServe(), async (client) => {
      const fromOption = [];
      const fromListener = [];
      const fromClient = [];
      client.on("event", (event) => fromClient.push(event.t));
      const execution = client.start(`events ${EVENT_COUNT}`, { onEvent: (event) => fromOption.push(event.t) });
      execution.on("event", (event) => fromListener.push(event.t));
      await execution.wait();
      assert.deepEqual(fromOption, EXPECTED_TYPES);
      assert.deepEqual(fromListener, EXPECTED_TYPES);
      assert.deepEqual(fromClient, EXPECTED_TYPES);
    });
  });

  it("delivers events in order of their sequence numbers", async () => {
    await withClient(fakeServe(), async (client) => {
      const sequence = [];
      await client.start(`events ${EVENT_COUNT}`, { onEvent: (event) => sequence.push(event.seq) }).wait();
      const sorted = [...sequence].sort((left, right) => left - right);
      assert.deepEqual(sequence, sorted);
      assert.equal(sequence.length, EXPECTED_TYPES.length);
    });
  });

  it("routes events only to the execution of their request", async () => {
    await withClient(fakeServe(), async (client) => {
      const first = [];
      const second = [];
      const a = client.start(`events ${EVENT_COUNT}`, { onEvent: (event) => first.push(event.request) });
      const b = client.start("events 1", { onEvent: (event) => second.push(event.request) });
      await Promise.all([a.wait(), b.wait()]);
      assert.equal(first.length, EXPECTED_TYPES.length);
      assert.equal(second.length, 2);
      assert.equal(new Set(first).size, 1);
      assert.equal(new Set(second).size, 1);
      assert.notEqual(first[0], second[0]);
    });
  });

  it("ends an iteration started after the execution has ended", async () => {
    await withClient(fakeServe(), async (client) => {
      const execution = client.start("echo done");
      await execution.wait();
      const events = [];
      for await (const event of execution) {
        events.push(event);
      }
      assert.deepEqual(events, []);
    });
  });
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
