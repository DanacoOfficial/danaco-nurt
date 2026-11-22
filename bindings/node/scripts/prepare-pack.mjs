// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
// Copies LICENSE and NOTICE of the repository into the main package and into
// every platform package under npm/, so that each packed tarball carries them.

import { copyFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const LEGAL_FILES = ["LICENSE", "NOTICE"];

const packageDirectory = join(dirname(fileURLToPath(import.meta.url)), "..");
const repositoryDirectory = join(packageDirectory, "..", "..");
const platformDirectory = join(packageDirectory, "npm");

const targets = [
  packageDirectory,
  ...readdirSync(platformDirectory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => join(platformDirectory, entry.name)),
];

for (const target of targets) {
  for (const file of LEGAL_FILES) {
    copyFileSync(join(repositoryDirectory, file), join(target, file));
  }
}
