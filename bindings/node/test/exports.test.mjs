// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
import assert from "node:assert/strict";
import { describe, it } from "node:test";
import * as esm from "../esm/index.mjs";
import { nurt as cjs } from "./support.mjs";

describe("entry points", () => {
  it("export the same names from the ES module and the CommonJS module", () => {
    const commonjs = Object.keys(cjs).filter((name) => name !== "__esModule").sort();
    assert.deepEqual(Object.keys(esm).sort(), commonjs);
  });

  it("share one implementation", () => {
    assert.equal(esm.Nurt, cjs.Nurt);
    assert.equal(esm.NurtError, cjs.NurtError);
    assert.ok(new esm.NurtError("NURT_E_ARG", "test") instanceof cjs.NurtError);
  });
});
