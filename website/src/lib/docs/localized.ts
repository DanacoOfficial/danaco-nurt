// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.localized
// Responsibility:   The documentation seen in one language: the title, the description and the address of each entry, taken from the Polish translation where one exists and from the English original otherwise.
// Dependencies:     website.config.i18n, website.src.lib.docs.model, website.src.lib.docs.translations, website.src.lib.urls

import type { Locale } from '../../../config/i18n';
import { entryUrl, type DocsEntry } from './model';
import { getDocsTranslations, type DocsTranslation } from './translations';
import { href } from '../urls';

export interface LocalizedDocs {
  readonly locale: Locale;
  readonly translations: ReadonlyMap<string, DocsTranslation>;
  title(entry: DocsEntry): string;
  description(entry: DocsEntry): string;
  url(entry: DocsEntry): string;
  translated(entry: DocsEntry): boolean;
}

/** Builds the view of the documentation in one language; English uses the originals only. */
export async function localizedDocs(locale: Locale): Promise<LocalizedDocs> {
  const translations = new Map<string, DocsTranslation>();
  if (locale !== 'en') {
    for (const translation of await getDocsTranslations()) translations.set(translation.id, translation);
  }
  return {
    locale,
    translations,
    title: (entry) => translations.get(entry.id)?.data.title ?? entry.data.title,
    description: (entry) => translations.get(entry.id)?.data.description ?? entry.data.description,
    url: (entry) => (translations.has(entry.id) ? href(entryUrl(entry), locale) : href(entryUrl(entry), 'en')),
    translated: (entry) => translations.has(entry.id),
  };
}
