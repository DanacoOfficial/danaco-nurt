// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.config.seo
// Responsibility:   Defaults of the page metadata: title pattern, descriptions per language, social card and robots rules.
// Dependencies:     website.config.i18n

import type { Locale } from './i18n';

export const seo = {
  titleSeparator: ' · ',
  socialCard: '/social/social-card.png',
  socialCardSize: { width: 1200, height: 630 },
  themeColor: { dark: '#0B0C0E', light: '#F6F7F8' },
  twitterCard: 'summary_large_image',
  descriptions: {
    en: 'Danaco Nurt is a command shell and language where every change can be undone, built for people and AI agents. One tool for Linux, Windows and macOS.',
    pl: 'Danaco Nurt to powłoka poleceń i język, w których każdą zmianę można cofnąć, stworzone dla ludzi i agentów AI. Jedno narzędzie dla Linuksa, Windows i macOS.',
  } satisfies Record<Locale, string>,
  robots: {
    allow: ['/'],
    disallow: [] as string[],
  },
} as const;
