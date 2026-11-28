// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.import-docs-translations
// Responsibility:   Copies the Polish translations of the documentation of the release into website/translations/pl/, each file at the repository path of its English original (docs/commands/replace.md, spec/grammar.md), so the documentation loader finds a translation by the path of the source. A delivered file replaces the file at its path; a translation written in the repository for a file outside the delivered set stays.
// Dependencies:     node:fs, node:path

import { copyFileSync, existsSync, mkdirSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const source = process.argv[2];
if (!source || !existsSync(source)) {
  throw new Error('import-docs-translations: give the directory of the translations as the first argument');
}
const target = path.join(root, 'translations/pl');

/** Lists the Markdown files under a directory with their paths relative to it. */
function walk(directory, prefix = '') {
  const files = [];
  for (const name of readdirSync(directory)) {
    const full = path.join(directory, name);
    const relative = prefix ? `${prefix}/${name}` : name;
    if (statSync(full).isDirectory()) files.push(...walk(full, relative));
    else if (name.endsWith('.md')) files.push({ full, relative });
  }
  return files;
}

let copied = 0;
const place = (full, relative) => {
  if (!relative.startsWith('docs/') && !relative.startsWith('spec/')) return;
  const destination = path.join(target, relative);
  mkdirSync(path.dirname(destination), { recursive: true });
  copyFileSync(full, destination);
  copied += 1;
};

const releaseDirectory = path.join(source, 'release-pl');
for (const batch of readdirSync(releaseDirectory)) {
  for (const file of walk(path.join(releaseDirectory, batch))) place(file.full, file.relative);
}
const commandDirectory = path.join(source, 'docs-pl');
for (const batch of readdirSync(commandDirectory)) {
  for (const file of walk(path.join(commandDirectory, batch))) place(file.full, `docs/${file.relative}`);
}
console.log(`import-docs-translations: ${copied} files in translations/pl`);
