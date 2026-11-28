// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.motion.targets
// Responsibility:   The elements that enter with motion. The same list is written in the style layer (motion.css), which hides these elements before their entrance; both lists must stay equal.
// Dependencies:     none.

export const revealSelectors: readonly string[] = [
  '.section__head',
  '.page-header__title',
  '.page-header__lead',
  '.card',
  '.stat',
  '.how__point',
  '.terminal',
  '.session',
  '.timeline > li',
  '.step',
  '.callout',
  '.figure',
  '.cta-band__inner',
  '.prose > h2',
  '.prose > table',
  '.prose > .code-block',
];
