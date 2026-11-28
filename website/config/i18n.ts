// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.config.i18n
// Responsibility:   Languages of the site: the list, the source language, the labels and the path prefix of each language.
// Dependencies:     none.

export const locales = ['en', 'pl'] as const;
export type Locale = (typeof locales)[number];

export const defaultLocale: Locale = 'en';

export const localeSettings: Record<Locale, { label: string; shortLabel: string; htmlLang: string; openGraph: string; prefix: string }> = {
  en: { label: 'English', shortLabel: 'EN', htmlLang: 'en', openGraph: 'en_US', prefix: '' },
  pl: { label: 'Polski', shortLabel: 'PL', htmlLang: 'pl', openGraph: 'pl_PL', prefix: 'pl' },
};

export function isLocale(value: string | undefined): value is Locale {
  return value !== undefined && (locales as readonly string[]).includes(value);
}
