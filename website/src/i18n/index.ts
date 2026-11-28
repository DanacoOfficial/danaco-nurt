// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.i18n.index
// Responsibility:   Access to the interface dictionary of a language and the formatting of dates in that language.
// Dependencies:     website.config.i18n, website.src.i18n.ui.en, website.src.i18n.ui.pl

import type { Locale } from '../../config/i18n';
import { en, type UiStrings } from './ui/en';
import { pl } from './ui/pl';

const dictionaries: Record<Locale, UiStrings> = { en, pl };

export function useUi(locale: Locale): UiStrings {
  return dictionaries[locale];
}

/** The word for pages after a count. Polish has one form for 1, one for counts ending in 2, 3 and 4 (except 12, 13 and 14) and one for the rest; English uses its plural for both. */
export function pageCountLabel(count: number, locale: Locale): string {
  const forms = dictionaries[locale].common.pageCount;
  if (count === 1) {
    return forms.one;
  }
  const few = count % 10 >= 2 && count % 10 <= 4 && !(count % 100 >= 12 && count % 100 <= 14);
  return few ? forms.few : forms.many;
}

export function formatDate(value: Date, locale: Locale): string {
  return new Intl.DateTimeFormat(locale === 'pl' ? 'pl-PL' : 'en-GB', {
    year: 'numeric',
    month: 'long',
    day: 'numeric',
    timeZone: 'UTC',
  }).format(value);
}

export function formatNumber(value: number, locale: Locale): string {
  return new Intl.NumberFormat(locale === 'pl' ? 'pl-PL' : 'en-GB').format(value);
}

export type { UiStrings };
