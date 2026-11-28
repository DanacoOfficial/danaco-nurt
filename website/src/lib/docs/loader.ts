// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.loader
// Responsibility:   Content loader of the documentation: reads the Markdown files of the repository directories named in the docs configuration and stores them as entries of the docs collection.
// Dependencies:     astro (loader interface), website.config.docs, website.src.lib.docs.frontmatter, website.src.lib.docs.links, website.src.lib.docs.urls, website.src.lib.docs.angleBrackets

import { readdir, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import type { Loader } from 'astro/loaders';
import { docsExcludedFiles, docsRoots, docsSources, type DocsSource } from '../../../config/docs';
import { docsUrl } from './urls';
import { readSource } from './frontmatter';
import { rewriteLinks, type UrlMap } from './links';
import { literalAngleBrackets } from './angle-brackets';

interface Discovered {
  readonly source: DocsSource;
  readonly repositoryPath: string;
  readonly stem: string;
  readonly id: string;
  readonly text: string;
}

function entryId(source: DocsSource, stem: string): string {
  if (source.sectionIndex === stem) {
    return `${source.section}/index`;
  }
  if (stem === 'README') {
    return `${source.section}/about`;
  }
  return `${source.section}/${stem}`;
}

async function discover(root: string): Promise<Discovered[]> {
  const found: Discovered[] = [];
  for (const source of docsSources) {
    const directory = path.resolve(root, docsRoots[source.root], source.directory);
    let names: string[];
    try {
      names = await readdir(directory);
    } catch {
      continue;
    }
    for (const name of names.sort()) {
      if (!name.endsWith('.md') || docsExcludedFiles.includes(name)) {
        continue;
      }
      const stem = name.slice(0, -3);
      const repositoryPath = [source.root, source.directory, name].filter(Boolean).join('/');
      found.push({
        source,
        repositoryPath,
        stem,
        id: entryId(source, stem),
        text: await readFile(path.join(directory, name), 'utf8'),
      });
    }
  }
  return found;
}

export function docsLoader(): Loader {
  return {
    name: 'nurt-docs-loader',
    async load({ config, store, parseData, renderMarkdown, generateDigest, logger }) {
      const root = fileURLToPath(config.root);
      const files = await discover(root);
      const urls = new Map<string, string>();
      for (const file of files) {
        urls.set(file.repositoryPath, docsUrl(file.id));
      }
      store.clear();
      for (const file of files) {
        const parsed = readSource(file.text, file.stem);
        const body = literalAngleBrackets(rewriteLinks(parsed.body, file.repositoryPath, urls as UrlMap));
        const order = typeof parsed.front.order === 'number' ? parsed.front.order : undefined;
        const data = await parseData({
          id: file.id,
          data: {
            title: parsed.title,
            description: parsed.description,
            section: file.source.section,
            sectionOrder: file.source.order,
            order,
            repositoryPath: file.repositoryPath,
            stem: file.stem,
            commands: Array.isArray(parsed.front.commands) ? parsed.front.commands.map(String) : undefined,
            summary: typeof parsed.front.summary === 'string' ? parsed.front.summary : undefined,
            group: typeof parsed.front.group === 'string' ? parsed.front.group : undefined,
            since: typeof parsed.front.since === 'string' || typeof parsed.front.since === 'number' ? String(parsed.front.since) : undefined,
          },
        });
        store.set({
          id: file.id,
          data,
          body,
          digest: generateDigest(file.text),
          rendered: await renderMarkdown(body),
        });
      }
      logger.info(`loaded ${files.length} documentation files`);
    },
  };
}

/** Directory of the Polish translations, laid out by the repository path of each English original. */
const translationsDirectory = 'translations/pl';

/** Turns a heading with an explicit identifier, "## Opis {#description}", into an HTML heading that keeps the identifier of the English original. */
function explicitHeadings(markdown: string): string {
  let fenced = false;
  return markdown
    .split('\n')
    .map((line) => {
      if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
      if (fenced) return line;
      const match = /^(#{2,4})\s+(.*?)\s*\{#([\w.-]+)\}\s*$/.exec(line);
      if (!match) {
        return line;
      }
      // The heading already gets the identifier made from its text; the explicit identifier is added as an anchor only when it differs, so no identifier repeats on the page.
      const slug = match[2]
        .replace(/`/g, '')
        .toLowerCase()
        .trim()
        .replace(/[^\p{L}\p{N}\s_-]/gu, '')
        .replace(/\s/g, '-');
      return slug === match[3] ? `${match[1]} ${match[2]}` : `${match[1]} <a id="${match[3]}"></a>${match[2]}`;
    })
    .join('\n');
}

/**
 * Content loader of the Polish documentation: for every documentation file that has a translation in translations/pl it stores an entry with the identifier of the English original,
 * with links rewritten to the Polish pages where a translation exists and to the English pages otherwise.
 */
export function docsTranslationsLoader(): Loader {
  return {
    name: 'nurt-docs-translations-loader',
    async load({ config, store, parseData, renderMarkdown, generateDigest, logger }) {
      const root = fileURLToPath(config.root);
      const files = await discover(root);
      const translated: { file: Discovered; text: string }[] = [];
      for (const file of files) {
        try {
          translated.push({ file, text: await readFile(path.join(root, translationsDirectory, file.repositoryPath), 'utf8') });
        } catch {
          continue;
        }
      }
      const available = new Set(translated.map((item) => item.file.repositoryPath));
      const urls = new Map<string, string>();
      for (const file of files) {
        urls.set(file.repositoryPath, available.has(file.repositoryPath) ? `/pl${docsUrl(file.id)}` : docsUrl(file.id));
      }
      store.clear();
      for (const { file, text } of translated) {
        const parsed = readSource(text, file.stem);
        const body = explicitHeadings(literalAngleBrackets(rewriteLinks(parsed.body, file.repositoryPath, urls as UrlMap)));
        const order = typeof parsed.front.order === 'number' ? parsed.front.order : undefined;
        const data = await parseData({
          id: file.id,
          data: {
            title: parsed.title,
            description: parsed.description,
            section: file.source.section,
            sectionOrder: file.source.order,
            order,
            repositoryPath: file.repositoryPath,
            stem: file.stem,
            commands: Array.isArray(parsed.front.commands) ? parsed.front.commands.map(String) : undefined,
            summary: typeof parsed.front.summary === 'string' ? parsed.front.summary : undefined,
            group: typeof parsed.front.group === 'string' ? parsed.front.group : undefined,
            since: typeof parsed.front.since === 'string' || typeof parsed.front.since === 'number' ? String(parsed.front.since) : undefined,
          },
        });
        store.set({ id: file.id, data, body, digest: generateDigest(text), rendered: await renderMarkdown(body) });
      }
      logger.info(`loaded ${translated.length} Polish documentation files`);
    },
  };
}
