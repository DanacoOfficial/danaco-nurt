// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.markdown.hast-base
// Responsibility:   Hast plugin of the Sätteri Markdown processor that puts the deployment base in front of the root-relative links and images of rendered Markdown, so that the same content works on a subdomain and on a path of the company domain.
// Dependencies:     satteri, website.config.site

import { defineHastPlugin } from 'satteri';
import { site } from '../../../config/site';

const ATTRIBUTES: Record<string, string> = { a: 'href', img: 'src' };
const base = site.base === '/' ? '' : site.base.replace(/\/$/, '');

export const baseLinks = defineHastPlugin({
  name: 'nurt-base-links',
  element: {
    filter: ['a', 'img'],
    visit(node, ctx) {
      if (base === '') {
        return;
      }
      const attribute = ATTRIBUTES[node.tagName];
      const properties = node.properties as Record<string, unknown> | undefined;
      const value = attribute && properties ? properties[attribute] : undefined;
      if (typeof value === 'string' && value.startsWith('/') && !value.startsWith('//') && !value.startsWith(`${base}/`)) {
        ctx.setProperty(node, attribute, `${base}${value}`);
      }
    },
  },
});
