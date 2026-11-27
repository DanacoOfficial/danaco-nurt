// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           editors/tree-sitter-nurt/test/config.test.js
// Responsibility:   Source file editors/tree-sitter-nurt/test/config.test.js.
// Dependencies:     none
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');

const root = path.resolve(__dirname, '..');
const config = JSON.parse(fs.readFileSync(path.join(root, 'tree-sitter.json'), 'utf8'));

test('every grammar of tree-sitter.json has a grammar file and its query files', () => {
  assert.deepEqual(
    config.grammars.map((grammar) => grammar.name),
    ['nurt', 'nurt_b', 'nurt_c'],
  );
  for (const grammar of config.grammars) {
    const directory = path.resolve(root, grammar.path);
    assert.ok(fs.existsSync(path.join(directory, 'grammar.js')), `${grammar.name}: grammar.js`);
    for (const key of ['highlights', 'injections']) {
      const file = path.resolve(directory, grammar[key]);
      assert.ok(fs.existsSync(file), `${grammar.name}: ${key} ${file}`);
    }
  }
});

test('the package lists the grammar files of every variant', () => {
  const manifest = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
  for (const grammar of config.grammars) {
    const entry = grammar.path === '.' ? 'grammar.js' : `${grammar.path}/grammar.js`;
    assert.ok(manifest.files.includes(entry), entry);
  }
});

test('the scopes and the injection patterns of the variants differ', () => {
  const scopes = config.grammars.map((grammar) => grammar.scope);
  assert.equal(new Set(scopes).size, scopes.length);
  const patterns = config.grammars.map((grammar) => grammar['injection-regex']);
  assert.equal(new Set(patterns).size, patterns.length);
});

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
