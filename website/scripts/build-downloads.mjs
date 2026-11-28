// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.build-downloads
// Responsibility:   Builds the downloads of the site before the site build: packs the directories named in config/downloads.json (the identity files of branding/ and the training packages of website/training) into ZIP archives in public/downloads, and writes public/downloads/manifest.json with the size, the number of files and the SHA-256 sum of every download. The archives are reproducible: the entries are sorted and carry the date of the configuration.
// Dependencies:     fflate (MIT), node:fs, node:path, node:crypto, website.config.downloads

import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { zipSync } from 'fflate';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const config = JSON.parse(readFileSync(path.join(root, 'config/downloads.json'), 'utf8'));
const output = path.join(root, 'public/downloads');
const archiveDate = new Date(config.archiveDate);
const stored = new Set(['.png', '.pdf', '.ico', '.icns', '.webp', '.zip', '.woff2']);

function filesOf(location) {
  if (statSync(location).isFile()) {
    return [location];
  }
  return readdirSync(location)
    .sort()
    .flatMap((name) => (name === '__pycache__' ? [] : filesOf(path.join(location, name))));
}

function sha256(data) {
  return createHash('sha256').update(data).digest('hex');
}

function archive(entry) {
  const entries = {};
  let count = 0;
  for (const source of entry.sources) {
    const location = path.resolve(root, source);
    if (!existsSync(location)) {
      throw new Error(`build-downloads: source not found: ${source}`);
    }
    const base = path.dirname(location);
    for (const file of filesOf(location)) {
      const name = path.relative(base, file).split(path.sep).join('/');
      const level = stored.has(path.extname(file).toLowerCase()) ? 0 : 9;
      entries[name] = [readFileSync(file), { level, mtime: archiveDate }];
      count += 1;
    }
  }
  const data = zipSync(entries);
  writeFileSync(path.join(output, entry.archive), data);
  return { path: `downloads/${entry.archive}`, bytes: data.length, sha256: sha256(data), files: count };
}

function existing(entry) {
  const location = path.join(root, 'public', entry.file);
  if (!existsSync(location)) {
    throw new Error(`build-downloads: file not found: public/${entry.file}`);
  }
  const data = readFileSync(location);
  return { path: entry.file, bytes: data.length, sha256: sha256(data), files: 1 };
}

rmSync(output, { recursive: true, force: true });
mkdirSync(output, { recursive: true });

const manifest = { format: 'nurt-downloads/1', date: config.archiveDate.slice(0, 10), downloads: {} };
let total = 0;
for (const [group, entries] of Object.entries(config.groups)) {
  for (const entry of entries) {
    const result = entry.archive ? archive(entry) : existing(entry);
    manifest.downloads[`${group}/${entry.id}`] = result;
    total += result.bytes;
  }
}
writeFileSync(path.join(output, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`build-downloads: ${Object.keys(manifest.downloads).length} downloads, ${(total / 1048576).toFixed(1)} MiB`);
