// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/test/support.mjs
// Responsibility:   Source file bindings/node/test/support.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// Helpers shared by the test files.

import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);

/** Directory of the package. */
export const packageDirectory = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Path of the test server. */
export const fakeServePath = join(packageDirectory, "test", "fixtures", "fake-serve.mjs");

/** Public exports of the CommonJS build. */
export const nurt = require("../dist/index.js");

/** Loads an internal module of the CommonJS build. */
export function internal(name) {
  return require(`../dist/${name}.js`);
}

/** Client options that run the test server with the given options. */
export function fakeServe(...serverArgs) {
  return { executable: process.execPath, args: [fakeServePath, ...serverArgs] };
}

/**
 * Runs a test body with a client of the test server and closes the client
 * afterwards.
 */
export async function withClient(options, body) {
  const client = new nurt.Nurt(options);
  try {
    return await body(client);
  } finally {
    await client.close();
  }
}

/** Resolves after `ms` milliseconds. */
export function delay(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms);
  });
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
