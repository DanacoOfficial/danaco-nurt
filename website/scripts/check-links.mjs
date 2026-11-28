// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.check-links
// Responsibility:   Checks every internal link and resource of the built site: each href and src that points into the site must name a file of dist, and each fragment must name an id of the target page, and a link of a Polish page must lead to the Polish version of a page that has one. External addresses are not fetched. The exit code is 1 when a link is broken.
// Dependencies:     node:fs, node:path, website.config.site (through the environment SITE_BASE)

import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const dist = path.join(root, 'dist');
const base = (process.env.SITE_BASE ?? '/').replace(/\/?$/, '/');

function htmlFiles(directory) {
  return readdirSync(directory).flatMap((name) => {
    const full = path.join(directory, name);
    if (statSync(full).isDirectory()) {
      return htmlFiles(full);
    }
    return name.endsWith('.html') ? [full] : [];
  });
}

const idCache = new Map();

function idsOf(file) {
  if (!idCache.has(file)) {
    const html = readFileSync(file, 'utf8');
    idCache.set(file, new Set(Array.from(html.matchAll(/\sid="([^"]+)"/g), (match) => match[1])));
  }
  return idCache.get(file);
}

function resolveTarget(pathname) {
  if (!pathname.startsWith(base)) {
    return null;
  }
  const relative = decodeURIComponent(pathname.slice(base.length));
  const direct = path.join(dist, relative);
  if (relative === '' || relative.endsWith('/')) {
    const index = path.join(direct, 'index.html');
    return existsSync(index) ? index : null;
  }
  return existsSync(direct) && statSync(direct).isFile() ? direct : null;
}

if (!existsSync(dist)) {
  console.error('check-links: dist does not exist; build the site first');
  process.exit(2);
}

const broken = [];
let checked = 0;

for (const file of htmlFiles(dist)) {
  const html = readFileSync(file, 'utf8');
  const page = `/${path.relative(dist, file).replace(/index\.html$/, '')}`;
  for (const match of html.matchAll(/\s(?:href|src)="([^"]+)"/g)) {
    const value = match[1].replaceAll('&amp;', '&');
    if (/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(value)) {
      continue;
    }
    checked += 1;
    const url = new URL(value, `https://site.invalid${page}`);
    if (url.pathname === page && url.hash) {
      if (!idsOf(file).has(decodeURIComponent(url.hash.slice(1)))) {
        broken.push(`${page}: missing fragment ${url.hash}`);
      }
      continue;
    }
    const target = resolveTarget(url.pathname);
    if (!target) {
      broken.push(`${page}: ${value}`);
      continue;
    }
    if (url.hash && target.endsWith('.html') && !idsOf(target).has(decodeURIComponent(url.hash.slice(1)))) {
      broken.push(`${page}: ${value} (missing fragment)`);
    }
  }
}

// A Polish page must link to the Polish version of a page that has one; the language switch and the alternates carry hreflang and are left out.
const polish = `${base}pl/`;
for (const file of htmlFiles(path.join(dist, 'pl'))) {
  const html = readFileSync(file, 'utf8');
  const page = `/${path.relative(dist, file).replace(/index\.html$/, '')}`;
  for (const tag of html.matchAll(/<a\b[^>]*>/g)) {
    if (/\shreflang=/.test(tag[0])) {
      continue;
    }
    const target = /\shref="([^"#?]*\/)(?:[#?][^"]*)?"/.exec(tag[0])?.[1];
    if (!target || !target.startsWith(base) || target.startsWith(polish)) {
      continue;
    }
    if (existsSync(path.join(dist, 'pl', target.slice(base.length), 'index.html'))) {
      broken.push(`${page}: ${target} has a Polish version that the link does not use`);
    }
  }
}

const unique = Array.from(new Set(broken)).sort();
for (const line of unique) {
  console.log(line);
}
console.log(`check-links: ${checked} internal links checked, ${unique.length} broken`);
process.exit(unique.length > 0 ? 1 : 0);
