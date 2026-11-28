// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.pages
// Responsibility:   Access to the authored pages: the pages of a language, the pages of a section in the order of the sections map, and the logical path of a page.
// Dependencies:     astro:content, website.config.sections, website.config.i18n

import { getCollection, type CollectionEntry } from 'astro:content';
import type { Locale } from '../../config/i18n';
import { sectionPages, type SectionKey } from '../../config/sections';

export type PageEntry = CollectionEntry<'pages'>;

export function pageLocale(entry: PageEntry): Locale {
  return entry.id.split('/')[0] as Locale;
}

export function pageSlug(entry: PageEntry): string {
  return entry.id.split('/').slice(1).join('/');
}

export function pagePath(entry: PageEntry): string {
  return `/${pageSlug(entry)}/`;
}

export async function getPages(locale: Locale): Promise<PageEntry[]> {
  const entries = await getCollection('pages', (entry) => entry.id.startsWith(`${locale}/`));
  return entries.sort((a, b) => a.data.order - b.data.order || a.id.localeCompare(b.id));
}

export async function getSectionPages(locale: Locale, section: SectionKey): Promise<PageEntry[]> {
  const pages = await getPages(locale);
  const order = sectionPages[section];
  return pages
    .filter((entry) => entry.data.section === section)
    .sort((a, b) => order.indexOf(pageSlug(a).split('/')[1]) - order.indexOf(pageSlug(b).split('/')[1]));
}

export async function findPage(locale: Locale, slug: string): Promise<PageEntry | undefined> {
  const pages = await getPages(locale);
  return pages.find((entry) => pageSlug(entry) === slug);
}
