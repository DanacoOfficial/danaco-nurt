// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.config.theme
// Responsibility:   Theme of the site: the modes, the storage key and the layout limits shared by scripts and styles.
// Dependencies:     none.

export const theme = {
  modes: ['dark', 'light'] as const,
  defaultMode: 'dark',
  storageKey: 'nurt-theme',
  attribute: 'data-theme',
  contentWidth: 1200,
  proseWidth: 720,
  breakpoints: { compact: 640, medium: 960, wide: 1200 },
} as const;
