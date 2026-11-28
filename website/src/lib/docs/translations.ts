// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.translations
// Responsibility:   Access to the Polish translations of imported documentation pages: which entries have one and the translation of an entry; a translation has the identifier of its English original.
// Dependencies:     astro:content

import { getCollection, type CollectionEntry } from 'astro:content';

export type DocsTranslation = CollectionEntry<'docsTranslations'>;

export function translationKey(entry: DocsTranslation): string {
  return entry.id;
}

export async function getDocsTranslations(): Promise<DocsTranslation[]> {
  return getCollection('docsTranslations');
}

export async function translatedDocsIds(): Promise<Set<string>> {
  const translations = await getDocsTranslations();
  return new Set(translations.map(translationKey));
}
