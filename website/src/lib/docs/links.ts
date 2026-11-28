// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.links
// Responsibility:   Rewriting of the links of an imported documentation file: links between documents become pages of the site, other repository links become links to the repository.
// Dependencies:     website.config.site

import { repository } from '../../../config/site';

export type UrlMap = ReadonlyMap<string, string>;

const LINK = /(\]\()([^)\s]+)((?:\s+"[^"]*")?\))/g;
const EXTERNAL = /^(?:[a-z][a-z0-9+.-]*:|\/\/|#)/i;

export function normalise(path: string): string {
  const parts: string[] = [];
  for (const part of path.split('/')) {
    if (part === '' || part === '.') {
      continue;
    }
    if (part === '..') {
      parts.pop();
    } else {
      parts.push(part);
    }
  }
  return parts.join('/');
}

function directoryOf(path: string): string {
  const index = path.lastIndexOf('/');
  return index < 0 ? '' : path.slice(0, index);
}

export function rewriteTarget(target: string, sourcePath: string, urls: UrlMap): string {
  if (EXTERNAL.test(target)) {
    return target;
  }
  const hashIndex = target.indexOf('#');
  const file = hashIndex < 0 ? target : target.slice(0, hashIndex);
  const fragment = hashIndex < 0 ? '' : target.slice(hashIndex);
  if (file === '') {
    return target;
  }
  const resolved = normalise(`${directoryOf(sourcePath)}/${file}`);
  const page = urls.get(resolved);
  if (page) {
    return `${page}${fragment}`;
  }
  const isDirectory = file.endsWith('/') || !/\.[A-Za-z0-9]+$/.test(file);
  const kind = isDirectory ? 'tree' : 'blob';
  return `${repository.url}/${kind}/main/${resolved}${fragment}`;
}

export function rewriteLinks(markdown: string, sourcePath: string, urls: UrlMap): string {
  let inFence = false;
  return markdown
    .split('\n')
    .map((line) => {
      if (line.trimStart().startsWith('```')) {
        inFence = !inFence;
        return line;
      }
      if (inFence) {
        return line;
      }
      return line.replace(LINK, (_whole, open: string, target: string, close: string) => `${open}${rewriteTarget(target, sourcePath, urls)}${close}`);
    })
    .join('\n');
}
