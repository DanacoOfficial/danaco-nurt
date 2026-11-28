// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.translatedPaths
// Responsibility:   The set of documentation addresses that have a Polish page: the page of every translated file in translations/pl and the landing page of every documentation section; read once from the file system when the site is built.
// Dependencies:     node:fs, node:path, website.config.docs, website.src.lib.docs.urls

import { existsSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { docsExcludedFiles, docsPathPrefix, docsRoots, docsSources } from '../../../config/docs';
import { docsUrl } from './urls';

/** Identifier of a documentation entry; the same rule as the documentation loader. */
export function docsEntryId(section: string, sectionIndex: string | undefined, stem: string): string {
  if (sectionIndex === stem) return `${section}/index`;
  if (stem === 'README') return `${section}/about`;
  return `${section}/${stem}`;
}

function collect(): Set<string> {
  const paths = new Set<string>();
  const root = process.cwd();
  for (const source of docsSources) {
    paths.add(`${docsPathPrefix}/${source.section}/`);
    const translated = path.join(root, 'translations/pl', source.root, source.directory);
    const original = path.resolve(root, docsRoots[source.root], source.directory);
    if (!existsSync(translated) || !existsSync(original)) continue;
    for (const name of readdirSync(translated)) {
      if (!name.endsWith('.md') || docsExcludedFiles.includes(name)) continue;
      if (!existsSync(path.join(original, name))) continue;
      paths.add(docsUrl(docsEntryId(source.section, source.sectionIndex, name.slice(0, -3))));
    }
  }
  return paths;
}

/** Documentation addresses that exist in Polish. */
export const translatedDocsPaths: ReadonlySet<string> = collect();
