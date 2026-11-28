// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.model
// Responsibility:   Model of the imported documentation: the entries, the sections in order, the entries of a section, the neighbours of a page and the command groups.
// Dependencies:     astro:content, website.config.docs, website.src.lib.docs.urls

import { getCollection, type CollectionEntry } from 'astro:content';
import { docsSources } from '../../../config/docs';
import { docsUrl } from './urls';

export type DocsEntry = CollectionEntry<'docs'>;

export interface DocsGroup {
  readonly section: string;
  readonly labelKey: string;
  readonly order: number;
  readonly entries: DocsEntry[];
}

export function entryUrl(entry: DocsEntry): string {
  return docsUrl(entry.id);
}

export function compareEntries(a: DocsEntry, b: DocsEntry): number {
  if (a.data.order !== undefined && b.data.order !== undefined && a.data.order !== b.data.order) {
    return a.data.order - b.data.order;
  }
  if (a.id.endsWith('/index')) {
    return -1;
  }
  if (b.id.endsWith('/index')) {
    return 1;
  }
  return a.data.title.localeCompare(b.data.title, 'en');
}

export async function getDocs(): Promise<DocsEntry[]> {
  return getCollection('docs');
}

export function groupDocs(entries: readonly DocsEntry[]): DocsGroup[] {
  return docsSources
    .map((source) => ({
      section: source.section,
      labelKey: source.labelKey,
      order: source.order,
      entries: entries.filter((entry) => entry.data.section === source.section).sort(compareEntries),
    }))
    .filter((group) => group.entries.length > 0);
}

export function neighbours(group: DocsGroup | undefined, entry: DocsEntry): { previous?: DocsEntry; next?: DocsEntry } {
  if (!group) {
    return {};
  }
  const index = group.entries.findIndex((candidate) => candidate.id === entry.id);
  return {
    previous: index > 0 ? group.entries[index - 1] : undefined,
    next: index >= 0 && index < group.entries.length - 1 ? group.entries[index + 1] : undefined,
  };
}

export function commandPrefix(entry: DocsEntry): string {
  const stem = entry.data.stem;
  const dot = stem.indexOf('.');
  return dot < 0 ? 'core' : stem.slice(0, dot);
}

export function groupCommands(entries: readonly DocsEntry[]): Map<string, DocsEntry[]> {
  const groups = new Map<string, DocsEntry[]>();
  for (const entry of entries.filter((candidate) => candidate.data.section === 'commands' && candidate.data.stem !== 'README')) {
    const key = commandPrefix(entry);
    groups.set(key, [...(groups.get(key) ?? []), entry]);
  }
  return new Map([...groups.entries()].sort(([a], [b]) => (a === 'core' ? -1 : b === 'core' ? 1 : a.localeCompare(b))));
}
