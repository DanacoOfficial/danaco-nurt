// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           bindings/node/esm/index.mjs
// Responsibility:   Source file bindings/node/esm/index.mjs.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

// ES module entry point. The implementation is the CommonJS build, so that
// both entry points share one set of classes and `instanceof` holds across them.
// The build is loaded with require, which gives the same exports object in
// Node.js, Bun and Deno regardless of their interpretation of `__esModule`.

import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const nurt = require("../dist/index.js");

export const { Execution, Nurt, NurtError, NurtLanguageError, Response } = nurt;

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
