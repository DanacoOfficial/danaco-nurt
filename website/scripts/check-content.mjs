// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.check-content
// Responsibility:   Checks the authored pages against the map of the site: every slug of config/sections.ts has a page in every locale and no page lies outside the map, the front matter names its own section, the description is 40 to 220 characters long, every related slug exists, and the prose of a page stays within 1000 words. The exit code is 1 when a check fails.
// Dependencies:     node:fs, node:path, yaml, website.config.sections

import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'yaml';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const pagesRoot = path.join(root, 'src/content/pages');
const locales = ['en', 'pl'];
const wordLimit = 1000;

function readSections() {
  const source = readFileSync(path.join(root, 'config/sections.ts'), 'utf8');
  const sections = {};
  for (const match of source.matchAll(/^\s*(\w+):\s*\[([^\]]*)\]/gm)) {
    sections[match[1]] = Array.from(match[2].matchAll(/'([^']+)'/g), (item) => item[1]);
  }
  return sections;
}

function files(directory) {
  if (!existsSync(directory)) {
    return [];
  }
  return readdirSync(directory).flatMap((name) => {
    const full = path.join(directory, name);
    return statSync(full).isDirectory() ? files(full) : name.endsWith('.mdx') ? [full] : [];
  });
}

function proseWords(body) {
  const prose = body
    .replace(/```[\s\S]*?```/g, ' ')
    .replace(/\{\/\*[\s\S]*?\*\/\}/g, ' ')
    .split('\n')
    .filter((line) => !line.trim().startsWith('|') && !/^\s*<\/?[A-Z]/.test(line) && !line.trim().startsWith('import '))
    .join(' ');
  return prose.split(/\s+/).filter((word) => /[\p{L}\p{N}]/u.test(word)).length;
}

const sections = readSections();
const expected = new Set(Object.entries(sections).flatMap(([section, slugs]) => slugs.map((slug) => `${section}/${slug}`)));
const problems = [];
const warnings = [];

for (const locale of locales) {
  const present = new Set();
  for (const file of files(path.join(pagesRoot, locale))) {
    const relative = path.relative(path.join(pagesRoot, locale), file).replace(/\.mdx$/, '');
    present.add(relative);
    const label = `${locale}/${relative}`;
    if (!expected.has(relative)) {
      problems.push(`${label}: not in config/sections.ts`);
    }
    const text = readFileSync(file, 'utf8');
    const match = text.match(/^---\n([\s\S]*?)\n---\n([\s\S]*)$/);
    if (!match) {
      problems.push(`${label}: no front matter`);
      continue;
    }
    let data;
    try {
      data = parse(match[1]);
    } catch (error) {
      problems.push(`${label}: front matter is not valid YAML (${error.message.split('\n')[0]})`);
      continue;
    }
    const [section] = relative.split('/');
    if (data.section !== section) {
      problems.push(`${label}: section is ${data.section}, expected ${section}`);
    }
    const length = String(data.description ?? '').length;
    if (length < 40 || length > 220) {
      problems.push(`${label}: description has ${length} characters (40 to 220)`);
    }
    for (const related of data.related ?? []) {
      if (!expected.has(related)) {
        problems.push(`${label}: related slug ${related} does not exist`);
      }
    }
    const words = proseWords(match[2]);
    if (words > wordLimit) {
      warnings.push(`${label}: ${words} words of prose (limit ${wordLimit})`);
    }
  }
  for (const slug of expected) {
    if (!present.has(slug)) {
      problems.push(`${locale}/${slug}: page missing`);
    }
  }
}

for (const line of warnings) {
  console.log(`warning ${line}`);
}
for (const line of problems) {
  console.log(`error ${line}`);
}
console.log(`check-content: ${expected.size} slugs × ${locales.length} locales, ${problems.length} errors, ${warnings.length} warnings`);
process.exit(problems.length > 0 ? 1 : 0);
