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
