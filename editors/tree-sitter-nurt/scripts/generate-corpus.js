// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
'use strict';

// Writes test/corpus/<group>.txt from the golden program files. Each program
// becomes one test case. The expected tree of a new case is a placeholder that
// `tree-sitter test --update` replaces after the parser has been generated.

const fs = require('node:fs');
const path = require('node:path');

const GRAMMAR_DIR = path.resolve(__dirname, '..');
const REPOSITORY_ROOT = path.resolve(GRAMMAR_DIR, '..', '..');
const CORPUS_DIR = path.join(GRAMMAR_DIR, 'test', 'corpus');
const GOLDEN_DIRS = [
  path.join(REPOSITORY_ROOT, 'spec', 'golden', 'ast'),
  path.join(REPOSITORY_ROOT, 'tests', 'd1', 'golden', 'ast'),
];
const PROGRAM_SUFFIX = '.nurt';
const CANONICAL_SUFFIX = '.canonical.nurt';
const PLACEHOLDER_TREE = '(source_file)';

function collectPrograms(directory) {
  if (!fs.existsSync(directory)) {
    return [];
  }
  const found = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      found.push(...collectPrograms(full));
    } else if (
      entry.name.endsWith(PROGRAM_SUFFIX) &&
      !entry.name.endsWith(CANONICAL_SUFFIX)
    ) {
      found.push(full);
    }
  }
  return found;
}

function groupOf(file) {
  const base = path.basename(file, PROGRAM_SUFFIX);
  const match = /^(task|example|extra)-/.exec(base);
  if (match) {
    return match[1];
  }
  return path.basename(path.dirname(file));
}

function renderCase(file) {
  const name = path.basename(file, PROGRAM_SUFFIX);
  const text = fs.readFileSync(file, 'utf8').replace(/\r\n/g, '\n').replace(/\n+$/, '');
  const rule = '='.repeat(Math.max(name.length, 20));
  return `${rule}\n${name}\n${rule}\n\n${text}\n\n---\n\n${PLACEHOLDER_TREE}\n`;
}

function main() {
  const groups = new Map();
  for (const directory of GOLDEN_DIRS) {
    for (const file of collectPrograms(directory).sort()) {
      const group = groupOf(file);
      if (!groups.has(group)) {
        groups.set(group, []);
      }
      groups.get(group).push(file);
    }
  }
  fs.mkdirSync(CORPUS_DIR, { recursive: true });
  for (const [group, files] of groups) {
    const target = path.join(CORPUS_DIR, `golden-${group}.txt`);
    fs.writeFileSync(target, files.map(renderCase).join('\n'));
  }
}

main();
