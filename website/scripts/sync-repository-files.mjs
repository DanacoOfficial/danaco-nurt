// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.sync-repository-files
// Responsibility:   Copies the files of the repository that the site publishes at fixed addresses into public/: the installers (install.sh and install.ps1 at the root of the site, the addresses of the quick install commands) and the JSON schemas of the formats (schema/<name>.schema.json, the addresses named by the identifiers of the schemas). The copies are generated and not committed.
// Dependencies:     node:fs, node:path

import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const repository = path.resolve(root, '..');
const installers = ['install.sh', 'install.ps1'];

function requireFile(location) {
  if (!existsSync(location)) {
    throw new Error(`sync-repository-files: file not found: ${path.relative(repository, location)}`);
  }
  return location;
}

for (const name of installers) {
  copyFileSync(requireFile(path.join(repository, 'packaging/install', name)), path.join(root, 'public', name));
}

const schemaSource = requireFile(path.join(repository, 'spec/schemas'));
const schemaTarget = path.join(root, 'public/schema');
rmSync(schemaTarget, { recursive: true, force: true });
mkdirSync(schemaTarget, { recursive: true });
let schemas = 0;
for (const name of readdirSync(schemaSource).filter((entry) => entry.endsWith('.schema.json'))) {
  copyFileSync(path.join(schemaSource, name), path.join(schemaTarget, name));
  schemas += 1;
}
console.log(`sync-repository-files: ${installers.length} installers, ${schemas} schemas`);
