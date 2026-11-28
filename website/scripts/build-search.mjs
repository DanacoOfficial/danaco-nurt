// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.build-search
// Responsibility:   Builds the search index of the site with the Pagefind program from the built pages in dist/.
// Dependencies:     node:child_process, the pagefind program on the path.

import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const dist = resolve(root, 'dist');
if (!existsSync(dist)) {
  console.error('dist/ does not exist: run the site build first');
  process.exit(1);
}
const result = spawnSync('pagefind', ['--site', dist, '--output-subdir', 'pagefind'], { stdio: 'inherit' });
if (result.error) {
  console.error(`the pagefind program could not be started: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
