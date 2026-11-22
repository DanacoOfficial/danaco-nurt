// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Sets the version of the main package, of its optional dependencies and of
// the platform packages.
//
//   node scripts/set-version.mjs 1.2.3

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const SEMVER = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$/;
const INDENT = 2;

const version = process.argv[2];
if (version === undefined || !SEMVER.test(version)) {
  process.stderr.write("usage: node scripts/set-version.mjs <version>; the version must be a semantic version\n");
  process.exit(2);
}

const packageDirectory = join(dirname(fileURLToPath(import.meta.url)), "..");
const platformDirectory = join(packageDirectory, "npm");

function update(file, change) {
  const manifest = JSON.parse(readFileSync(file, "utf8"));
  change(manifest);
  writeFileSync(file, `${JSON.stringify(manifest, null, INDENT)}\n`);
}

update(join(packageDirectory, "package.json"), (manifest) => {
  manifest.version = version;
  for (const name of Object.keys(manifest.optionalDependencies)) {
    manifest.optionalDependencies[name] = version;
  }
});
for (const entry of readdirSync(platformDirectory, { withFileTypes: true })) {
  if (entry.isDirectory()) {
    update(join(platformDirectory, entry.name, "package.json"), (manifest) => {
      manifest.version = version;
    });
  }
}
