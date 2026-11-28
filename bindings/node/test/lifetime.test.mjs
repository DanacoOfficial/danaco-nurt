// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/test/lifetime.test.mjs
// Responsibility:   Source file bindings/node/test/lifetime.test.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { describe, it } from "node:test";
import { pathToFileURL } from "node:url";
import { fakeServePath, packageDirectory } from "./support.mjs";

/** Upper bound for a script that exits on its own. */
const EXIT_TIMEOUT_MS = 20000;

describe("lifetime of the server process", () => {
  it("lets a script exit after its calls without closing the client", () => {
    const script = `
      const { Nurt } = await import(${JSON.stringify(pathToFileURL(`${packageDirectory}/esm/index.mjs`).href)});
      const client = new Nurt({ executable: process.execPath, args: [${JSON.stringify(fakeServePath)}] });
      const response = await client.run("echo idle");
      process.stdout.write(response.text);
    `;
    const result = spawnSync(process.execPath, ["--input-type=module", "--eval", script], {
      encoding: "utf8",
      timeout: EXIT_TIMEOUT_MS,
    });
    assert.equal(result.error, undefined);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout, "idle");
  });
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
