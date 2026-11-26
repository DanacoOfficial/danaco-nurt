// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
'use strict';

// Builds the tree-sitter test corpus of each syntax variant from the golden
// program files, one test case per program.
//
// Without arguments the script writes `golden-<group>.txt` files whose
// expected trees are placeholders; `tree-sitter test --update` run in the
// grammar directory of the variant replaces them with the parsed trees.
//
// With `--check` the script writes nothing. It fails when a golden program
// has no corpus case with the same input, when a corpus case has no golden
// program, or when an expected tree is a placeholder or contains an `ERROR`
// or `MISSING` node.

const fs = require('node:fs');
const path = require('node:path');

const GRAMMAR_DIR = path.resolve(__dirname, '..');
const REPOSITORY_ROOT = path.resolve(GRAMMAR_DIR, '..', '..');
const SPEC_GOLDEN = path.join('spec', 'golden', 'ast');
const DOMAIN_GOLDEN = path.join('tests', 'd1', 'golden', 'ast');
const PROGRAM_SUFFIX = '.nurt';
const PLACEHOLDER_TREE = '(source_file)';
const CORPUS_PREFIX = 'golden-';
const RULE_MIN_WIDTH = 20;

const VARIANT_SUFFIX = /\.([bc])\.nurt$/;

const TARGETS = [
  {
    variant: 'a',
    grammarDir: GRAMMAR_DIR,
    sources: [
      { dir: SPEC_GOLDEN, accept: (name) => !VARIANT_SUFFIX.test(name) },
      { dir: path.join(DOMAIN_GOLDEN, 'a'), accept: () => true, group: 'canonical' },
    ],
  },
  {
    variant: 'b',
    grammarDir: path.join(GRAMMAR_DIR, 'variant-b'),
    sources: [
      { dir: SPEC_GOLDEN, accept: (name) => name.endsWith('.b.nurt') },
      { dir: path.join(DOMAIN_GOLDEN, 'b'), accept: () => true },
    ],
  },
  {
    variant: 'c',
    grammarDir: path.join(GRAMMAR_DIR, 'variant-c'),
    sources: [
      { dir: SPEC_GOLDEN, accept: (name) => name.endsWith('.c.nurt') },
      { dir: path.join(DOMAIN_GOLDEN, 'c'), accept: () => true },
    ],
  },
];

function normalize(text) {
  return text.replace(/\r\n/g, '\n').replace(/\n+$/, '');
}

function programsOf(target) {
  const programs = [];
  for (const source of target.sources) {
    const directory = path.join(REPOSITORY_ROOT, source.dir);
    if (!fs.existsSync(directory)) {
      continue;
    }
    const names = fs
      .readdirSync(directory, { withFileTypes: true })
      .filter((entry) => entry.isFile() && entry.name.endsWith(PROGRAM_SUFFIX))
      .map((entry) => entry.name)
      .filter(source.accept)
      .sort();
    for (const name of names) {
      const relative = path.join(source.dir, name);
      const group = source.group || /^(task|example|extra)-/.exec(name)?.[1] || 'other';
      programs.push({
        name: relative.split(path.sep).join('/'),
        group,
        text: normalize(fs.readFileSync(path.join(REPOSITORY_ROOT, relative), 'utf8')),
      });
    }
  }
  return programs;
}

function renderCase(program) {
  const rule = '='.repeat(Math.max(program.name.length, RULE_MIN_WIDTH));
  return `${rule}\n${program.name}\n${rule}\n\n${program.text}\n\n---\n\n${PLACEHOLDER_TREE}\n`;
}

function readCases(corpusDir) {
  const cases = new Map();
  if (!fs.existsSync(corpusDir)) {
    return cases;
  }
  const header = /^(={3,})\n([^\n]+)\n\1\n\n([\s\S]*?)\n\n-{3,}\n\n([\s\S]*?)(?=\n={3,}\n[^\n]+\n={3,}\n|(?![\s\S]))/gm;
  for (const file of fs.readdirSync(corpusDir).filter((name) => name.startsWith(CORPUS_PREFIX))) {
    const text = fs.readFileSync(path.join(corpusDir, file), 'utf8').replace(/\r\n/g, '\n');
    for (const match of text.matchAll(header)) {
      cases.set(match[2], { file, input: normalize(match[3]), tree: match[4].trim() });
    }
  }
  return cases;
}

function write(target) {
  const groups = new Map();
  for (const program of programsOf(target)) {
    if (!groups.has(program.group)) {
      groups.set(program.group, []);
    }
    groups.get(program.group).push(program);
  }
  const corpusDir = path.join(target.grammarDir, 'test', 'corpus');
  fs.mkdirSync(corpusDir, { recursive: true });
  for (const [group, programs] of groups) {
    const file = path.join(corpusDir, `${CORPUS_PREFIX}${group}.txt`);
    fs.writeFileSync(file, programs.map(renderCase).join('\n'));
  }
  return [...groups.values()].reduce((sum, programs) => sum + programs.length, 0);
}

function check(target) {
  const problems = [];
  const programs = programsOf(target);
  const cases = readCases(path.join(target.grammarDir, 'test', 'corpus'));
  for (const program of programs) {
    const found = cases.get(program.name);
    if (!found) {
      problems.push(`${program.name}: no corpus case`);
    } else if (found.input !== program.text) {
      problems.push(`${program.name}: corpus input differs from the golden program`);
    } else if (found.tree === PLACEHOLDER_TREE) {
      problems.push(`${program.name}: expected tree is a placeholder`);
    } else if (/\((?:ERROR|MISSING)\b/.test(found.tree)) {
      problems.push(`${program.name}: expected tree contains an error node`);
    }
  }
  const known = new Set(programs.map((program) => program.name));
  for (const name of cases.keys()) {
    if (!known.has(name)) {
      problems.push(`${name}: corpus case without a golden program`);
    }
  }
  return { count: programs.length, problems };
}

function main(argv) {
  const checking = argv.includes('--check');
  let failed = false;
  for (const target of TARGETS) {
    if (!checking) {
      const count = write(target);
      process.stdout.write(`variant ${target.variant}: ${count} cases written\n`);
      continue;
    }
    const { count, problems } = check(target);
    problems.forEach((problem) => process.stderr.write(`variant ${target.variant}: ${problem}\n`));
    failed = failed || problems.length > 0 || count === 0;
    process.stdout.write(`variant ${target.variant}: ${count} golden programs, ${problems.length} problems\n`);
  }
  if (failed) {
    process.exitCode = 1;
  }
}

main(process.argv.slice(2));
