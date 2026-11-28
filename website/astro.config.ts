// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.astro.config
// Responsibility:   Wiring of the generator: the address and base of the deployment, the integrations, the Markdown pipeline with the Nurt grammar and the build output.
// Dependencies:     astro, @astrojs/mdx, @astrojs/sitemap, @astrojs/markdown-satteri, website.config.site, website.config.i18n

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'astro/config';
import mdx from '@astrojs/mdx';
import sitemap from '@astrojs/sitemap';
import { site } from './config/site';
import { defaultLocale, localeSettings, locales } from './config/i18n';
import { satteri } from '@astrojs/markdown-satteri';
import { baseLinks } from './src/lib/markdown/hast-base';

const nurtGrammar = { ...JSON.parse(readFileSync(fileURLToPath(new URL('../editors/textmate/nurt.tmLanguage.json', import.meta.url)), 'utf8')), name: 'nurt' };

export default defineConfig({
  site: site.url,
  base: site.base,
  trailingSlash: 'always',
  output: 'static',
  build: { format: 'directory', assets: '_assets' },
  compressHTML: true,
  devToolbar: { enabled: false },
  integrations: [
    mdx(),
    sitemap({
      i18n: {
        defaultLocale,
        locales: Object.fromEntries(locales.map((locale) => [locale, localeSettings[locale].htmlLang])),
      },
      filter: (page) => !page.includes('/404') && !page.includes('/search/'),
    }),
  ],
  markdown: {
    syntaxHighlight: 'shiki',
    shikiConfig: {
      theme: 'github-dark-default',
      langs: ['sh', 'bash', 'json', 'toml', 'yaml', 'rust', 'c', 'python', 'powershell', 'text', 'ini', 'diff', 'markdown', nurtGrammar],
      wrap: false,
    },
    processor: satteri({ hastPlugins: [baseLinks] }),
  },
  vite: {
    build: { assetsInlineLimit: 0 },
  },
});
