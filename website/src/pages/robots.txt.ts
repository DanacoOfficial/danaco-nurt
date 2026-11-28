// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.pages.robots.txt
// Responsibility:   The robots file of the site: the rules from the SEO configuration and the address of the sitemap index.
// Dependencies:     website.config.site, website.config.seo, website.src.lib.urls

import type { APIRoute } from 'astro';
import { site } from '../../config/site';
import { seo } from '../../config/seo';
import { withBase } from '../lib/urls';

export const GET: APIRoute = () => {
  const lines = [
    'User-agent: *',
    ...seo.robots.allow.map((path) => `Allow: ${withBase(path)}`),
    ...seo.robots.disallow.map((path) => `Disallow: ${withBase(path)}`),
    '',
    `Sitemap: ${site.url}${withBase('/sitemap-index.xml')}`,
    '',
  ];
  return new Response(lines.join('\n'), { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
