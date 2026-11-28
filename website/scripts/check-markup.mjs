// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.check-markup
// Responsibility:   Checks the markup of the built site for three defects a browser hides: an element whose name is not an element of HTML or SVG (text in angle brackets read as a tag, which removes the text from the page), content written after the end of the document, and a backtick left in the text of a page (Markdown that was not read). A full check of the markup is the task of an HTML checker; this check needs no program outside the package. An argument names another directory than dist. Elements of HTML that the site does not use and the documentation writes as types (map, object, data) are not in the list.
// Dependencies:     node:fs, node:path

import { readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const dist = process.argv[2] ? path.resolve(process.argv[2]) : path.join(root, 'dist');

const HTML_ELEMENTS =
  'a abbr address area article aside audio b bdi bdo blockquote body br button canvas caption cite code col colgroup datalist dd del details dfn dialog div dl dt em fieldset figcaption figure footer form h1 h2 h3 h4 h5 h6 head header hgroup hr html i iframe img input ins kbd label legend li link main mark meta meter nav noscript ol optgroup option p picture pre progress q rp rt ruby s samp script section select small source span strong style sub summary sup table tbody td textarea tfoot th thead time title tr track u ul var video wbr';
const SVG_ELEMENTS =
  'circle clipPath defs desc ellipse feBlend feColorMatrix feComposite feFlood feGaussianBlur feMerge feMergeNode feOffset filter foreignObject g image line linearGradient marker mask path pattern polygon polyline radialGradient rect stop svg symbol text textPath tspan use';
const known = new Set(`${HTML_ELEMENTS} ${SVG_ELEMENTS}`.split(' '));
// The names of SVG that are also written as types in the documentation count as elements only inside an svg element.
const svgOnly = new Set(SVG_ELEMENTS.split(' '));

/** Lists the HTML files of a directory; the search index keeps fragments of pages and is skipped. */
function htmlFiles(directory) {
  return readdirSync(directory).flatMap((name) => {
    const full = path.join(directory, name);
    if (statSync(full).isDirectory()) {
      return name === 'pagefind' ? [] : htmlFiles(full);
    }
    return name.endsWith('.html') ? [full] : [];
  });
}

const errors = [];
const files = htmlFiles(dist);
for (const file of files) {
  const address = path.relative(dist, file);
  const html = readFileSync(file, 'utf8')
    .replace(/<!--[\s\S]*?-->/g, '')
    .replace(/<(script|style)\b[\s\S]*?<\/\1>/g, '<$1></$1>');
  const outside = html.replace(/<svg\b[\s\S]*?<\/svg>/g, '<svg></svg>');
  const unknown = new Set();
  for (const match of html.matchAll(/<([A-Za-z][A-Za-z0-9-]*)/g)) {
    if (!known.has(match[1])) unknown.add(match[1]);
  }
  for (const match of outside.matchAll(/<([A-Za-z][A-Za-z0-9-]*)/g)) {
    if (match[1] !== 'svg' && svgOnly.has(match[1])) unknown.add(match[1]);
  }
  for (const name of unknown) {
    errors.push(`${address}: <${name}> is not an element; text in angle brackets was read as a tag`);
  }
  if (!/<\/html>\s*$/.test(html)) {
    errors.push(`${address}: content after the end of the document`);
  }
  // A backtick in the text of the page, outside code, is Markdown that was not read: a code span cut by a table bar or left open.
  const text = (/<main\b[\s\S]*<\/main>/.exec(outside)?.[0] ?? '').replace(/<(pre|code)\b[\s\S]*?<\/\1>/g, ' ').replace(/<[^>]+>/g, ' ');
  const tick = text.indexOf('`');
  if (tick !== -1) {
    errors.push(`${address}: a backtick in the text, Markdown was not read: ${text.slice(Math.max(0, tick - 30), tick + 30).replace(/\s+/g, ' ').trim()}`);
  }
}

for (const error of errors) {
  console.error(`error ${error}`);
}
console.log(`check-markup: ${files.length} pages checked, ${errors.length} errors`);
process.exit(errors.length > 0 ? 1 : 0);
