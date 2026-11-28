// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.check-i18n
// Responsibility:   Checks that the Polish pages are a translation of the English pages: the same set of files, the same front matter fields that must agree, the same structure (headings, code blocks, components) and the right language prefix in internal links.
// Dependencies:     node:fs, yaml

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'yaml';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const pagesRoot = join(root, 'src/content/pages');

function files(directory) {
  return readdirSync(directory).flatMap((name) => {
    const path = join(directory, name);
    return statSync(path).isDirectory() ? files(path) : [path];
  });
}

function read(path) {
  const text = readFileSync(path, 'utf8');
  const match = /^---\n([\s\S]*?)\n---\n/.exec(text);
  const front = match ? parse(match[1]) : {};
  const body = match ? text.slice(match[0].length) : text;
  return { front, body };
}

function outline(body) {
  const withoutFences = body.replace(/```[\s\S]*?```/g, (block) => `\u0000FENCE:${block.split('\n')[0]}\u0000`);
  return {
    headings: (withoutFences.match(/^#{2,4} /gm) ?? []).map((heading) => heading.trim().length),
    fences: (body.match(/^```/gm) ?? []).length / 2,
    components: [...body.matchAll(/<([A-Z][A-Za-z]+)/g)].map((match) => match[1]).sort().join(','),
  };
}

function internalLinks(body) {
  const links = [...body.matchAll(/\]\((\/[^)\s]*)\)/g)].map((match) => match[1]);
  const props = [...body.matchAll(/href="(\/[^"]*)"/g)].map((match) => match[1]);
  return [...links, ...props];
}

const problems = [];
const english = new Map();
const polish = new Map();
for (const path of files(pagesRoot).filter((file) => file.endsWith('.mdx'))) {
  const key = relative(pagesRoot, path);
  const [language, ...rest] = key.split('/');
  (language === 'en' ? english : polish).set(rest.join('/'), path);
}

for (const key of english.keys()) {
  if (!polish.has(key)) {
    problems.push(`no Polish translation of ${key}`);
  }
}
for (const key of polish.keys()) {
  if (!english.has(key)) {
    problems.push(`Polish page without an English source: ${key}`);
  }
}

for (const [key, englishPath] of english) {
  const polishPath = polish.get(key);
  if (!polishPath) {
    continue;
  }
  const source = read(englishPath);
  const target = read(polishPath);
  for (const field of ['section', 'order', 'toc']) {
    if (JSON.stringify(source.front[field]) !== JSON.stringify(target.front[field])) {
      problems.push(`${key}: the field "${field}" differs between the languages`);
    }
  }
  if (JSON.stringify(source.front.related ?? []) !== JSON.stringify(target.front.related ?? [])) {
    problems.push(`${key}: the field "related" differs between the languages`);
  }
  const a = outline(source.body);
  const b = outline(target.body);
  if (JSON.stringify(a.headings) !== JSON.stringify(b.headings)) {
    problems.push(`${key}: the headings differ in number or level (${a.headings.length} and ${b.headings.length})`);
  }
  if (a.fences !== b.fences) {
    problems.push(`${key}: the number of code blocks differs (${a.fences} and ${b.fences})`);
  }
  if (a.components !== b.components) {
    problems.push(`${key}: the components used differ`);
  }
  for (const link of internalLinks(source.body)) {
    if (link.startsWith('/pl/')) {
      problems.push(`${key}: the English page links to a Polish address ${link}`);
    }
  }
  for (const link of internalLinks(target.body)) {
    if (!link.startsWith('/pl/') && !link.startsWith('/docs/') && !link.startsWith('/brand/')) {
      problems.push(`${key}: the Polish page links to ${link} without the /pl/ prefix`);
    }
  }
  if (source.front.title === target.front.title && !/^[A-Z0-9][\w .+-]*$/.test(source.front.title ?? '')) {
    problems.push(`${key}: the title is not translated`);
  }
}

if (problems.length > 0) {
  console.error(problems.join('\n'));
  console.error(`${problems.length} problems`);
  process.exit(1);
}
console.log(`i18n check: ${english.size} English pages, ${polish.size} Polish pages, no problems`);
