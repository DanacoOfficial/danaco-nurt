// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.config.search
// Responsibility:   Settings of the site search: the index directory, the content selector and the result limits.
// Dependencies:     none.

export const search = {
  outputDirectory: 'pagefind',
  contentSelector: '[data-pagefind-body]',
  resultsPerPage: 8,
  excerptLength: 24,
  minimumQueryLength: 2,
} as const;
