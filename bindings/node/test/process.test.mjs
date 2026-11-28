// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/test/process.test.mjs
// Responsibility:   Source file bindings/node/test/process.test.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import assert from "node:assert/strict";
import { once } from "node:events";
import { describe, it } from "node:test";
import { fakeServe, withClient } from "./support.mjs";

describe("supervision of the server process", () => {
  it("fails the calls in progress when the server exits and restarts it on the next call", async () => {
    await withClient(fakeServe(), async (client) => {
      const exited = once(client, "exit");
      const slow = client.run("delay 10000 never");
      const crash = client.run("crash");
      for (const call of [slow, crash]) {
        await assert.rejects(call, (/** @type {any} */ error) => {
          assert.equal(error.name, "NurtError");
          assert.equal(error.status, "NURT_E_INTERNAL");
          assert.match(error.message, /exited with code 3/);
          assert.match(error.message, /simulated failure/);
          return true;
        });
      }
      const [exitError] = await exited;
      assert.equal(exitError.status, "NURT_E_INTERNAL");
      const response = await client.run("echo again");
      assert.equal(response.text, "again");
    });
  });

  it("fails on a message written over several lines and restarts", async () => {
    await withClient(fakeServe(), async (client) => {
      await assert.rejects(client.run("noise"), (/** @type {any} */ error) => {
        assert.equal(error.status, "NURT_E_INTERNAL");
        assert.match(error.message, /invalid message from the server/);
        return true;
      });
      assert.equal((await client.run("echo after")).text, "after");
    });
  });

  it("fails on a message that is not UTF-8", async () => {
    await withClient(fakeServe(), async (client) => {
      await assert.rejects(client.run("binary"), /not valid UTF-8/);
    });
  });

  it("assembles a response delivered in several reads", async () => {
    await withClient(fakeServe(), async (client) => {
      assert.equal((await client.run("split")).text, "split");
    });
  });

  it("delivers several messages that arrive in one read", async () => {
    await withClient(fakeServe(), async (client) => {
      assert.equal((await client.run("events 5")).text, "events 5");
    });
  });
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
