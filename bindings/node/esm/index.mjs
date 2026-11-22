// ES module entry point. The implementation is the CommonJS build, so that
// both entry points share one set of classes and `instanceof` holds across them.
// The build is loaded with require, which gives the same exports object in
// Node.js, Bun and Deno regardless of their interpretation of `__esModule`.

import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const nurt = require("../dist/index.js");

export const { Execution, Nurt, NurtError, NurtLanguageError, Response } = nurt;
