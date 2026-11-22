// Runs test/*.test.mjs with the test runner of Node.js. The files are listed
// explicitly: the default discovery of the runner differs between Node.js
// versions and would also load the helpers in test/. Arguments are passed to
// the runner, for example --experimental-test-coverage.

import { spawnSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const testDirectory = join(dirname(fileURLToPath(import.meta.url)), "..", "test");
const files = readdirSync(testDirectory)
  .filter((name) => name.endsWith(".test.mjs"))
  .sort()
  .map((name) => join(testDirectory, name));

const result = spawnSync(process.execPath, ["--test", ...process.argv.slice(2), ...files], { stdio: "inherit" });
process.exitCode = result.status ?? 1;
