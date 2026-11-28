// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.pages.site.webmanifest
// Responsibility:   The web app manifest of the site, written with the deployment base so that it is correct on a subdomain and on a path.
// Dependencies:     website.config.site, website.config.seo, website.src.lib.urls

import type { APIRoute } from 'astro';
import { site } from '../../config/site';
import { seo } from '../../config/seo';
import { withBase } from '../lib/urls';

export const GET: APIRoute = () => {
  const manifest = {
    name: site.name,
    short_name: 'Nurt',
    description: `${site.tagline}.`,
    start_url: withBase('/'),
    scope: withBase('/'),
    display: 'standalone',
    theme_color: seo.themeColor.dark,
    background_color: seo.themeColor.dark,
    icons: [
      { src: withBase('/icon-192.png'), sizes: '192x192', type: 'image/png', purpose: 'any' },
      { src: withBase('/icon-512.png'), sizes: '512x512', type: 'image/png', purpose: 'any' },
      { src: withBase('/icon-maskable-512.png'), sizes: '512x512', type: 'image/png', purpose: 'maskable' },
    ],
  };
  return new Response(JSON.stringify(manifest, null, 2), { headers: { 'Content-Type': 'application/manifest+json' } });
};
