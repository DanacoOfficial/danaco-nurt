// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.urls
// Responsibility:   Addresses of the site: the logical path of a page, its address under the deployment base, its address in each language and its absolute address.
// Dependencies:     website.config.site, website.config.i18n

import { site } from '../../config/site';
import { defaultLocale, localeSettings, locales, type Locale } from '../../config/i18n';
import { docsPathPrefix } from '../../config/docs';
import { translatedDocsPaths } from './docs/translated-paths';

export function withBase(path: string): string {
  const base = site.base === '/' ? '' : site.base.replace(/\/$/, '');
  const normalised = path.startsWith('/') ? path : `/${path}`;
  return `${base}${normalised}`;
}

// A documentation page without a Polish translation keeps its English address in every locale; the hub, the section pages and every translated page have a Polish address.
function isEnglishOnly(path: string): boolean {
  return path.startsWith(`${docsPathPrefix}/`) && path !== `${docsPathPrefix}/` && !translatedDocsPaths.has(path.replace(/#.*$/, ''));
}

export function localisedPath(path: string, locale: Locale): string {
  const prefix = localeSettings[locale].prefix;
  const normalised = path.startsWith('/') ? path : `/${path}`;
  if (prefix === '' || isEnglishOnly(normalised)) {
    return normalised;
  }
  return `/${prefix}${normalised === '/' ? '/' : normalised}`;
}

export function href(path: string, locale: Locale = defaultLocale): string {
  return withBase(localisedPath(path, locale));
}

export function absoluteUrl(path: string, locale: Locale = defaultLocale): string {
  return `${site.url}${href(path, locale)}`.replace(/(?<!:)\/{2,}/g, '/');
}

export function logicalPath(pathname: string): { locale: Locale; path: string } {
  const base = site.base === '/' ? '' : site.base.replace(/\/$/, '');
  let rest = pathname.startsWith(base) ? pathname.slice(base.length) : pathname;
  if (!rest.startsWith('/')) {
    rest = `/${rest}`;
  }
  for (const locale of locales) {
    const prefix = localeSettings[locale].prefix;
    if (prefix !== '' && (rest === `/${prefix}` || rest.startsWith(`/${prefix}/`))) {
      const remainder = rest.slice(prefix.length + 1);
      return { locale, path: remainder === '' ? '/' : remainder };
    }
  }
  return { locale: defaultLocale, path: rest };
}

export function alternates(path: string): { locale: Locale; htmlLang: string; href: string; absolute: string }[] {
  return locales.map((locale) => ({
    locale,
    htmlLang: localeSettings[locale].htmlLang,
    href: href(path, locale),
    absolute: absoluteUrl(path, locale),
  }));
}
