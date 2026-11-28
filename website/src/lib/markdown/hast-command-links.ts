// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.markdown.hast-command-links
// Responsibility:   Hast plugin of the Sätteri Markdown processor that links a command name written as inline code to the page of that command in the language of the document. A name with a group (git.status) is linked wherever it stands; a name without a group (find) is also an ordinary word, a parameter or a value, so it is linked only in running text, not in a table, and two names that mostly stand for a value or a field are left out. Code in a code block, in a link or a linked card, in a heading and on the page of the command itself stays as it is.
// Dependencies:     satteri, node:fs, node:path, website.config.site

import { existsSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { defineHastPlugin } from 'satteri';
import { site } from '../../../config/site';

/** The marker a loader writes into a document it renders without a file address: the language and the identifier of the page. */
export function documentMarker(locale: 'en' | 'pl', id: string): string {
  return `<!--nurt-document:${locale}:${id}-->`;
}

/** True when a node stands inside a link: an anchor of the document or a component that renders its content as one link. */
export function insideLink(node: unknown, parentOf: (node: never) => unknown): boolean {
  for (let current = parentOf(node as never) as { type?: string; tagName?: string; name?: string | null } | undefined; current; current = parentOf(current as never) as typeof current) {
    if (current.type === 'element' && current.tagName === 'a') {
      return true;
    }
    if ((current.type === 'mdxJsxFlowElement' || current.type === 'mdxJsxTextElement') && current.name === 'Card') {
      return true;
    }
  }
  return false;
}

const base = site.base === '/' ? '' : site.base.replace(/\/$/, '');
const directory = path.resolve(process.cwd(), '..', 'docs', 'commands');
const commands = new Set(
  existsSync(directory)
    ? readdirSync(directory)
        .filter((name) => name.endsWith('.md') && name !== 'README.md' && name !== '_template.md')
        .map((name) => name.slice(0, -3))
    : [],
);

const PROSE = new Set(['p', 'li', 'em', 'strong', 'dd', 'blockquote']);
// Names that the documentation uses more often as the value of a parameter or as the name of a field than as the command.
const AMBIGUOUS = new Set(['skip', 'help']);
const NEVER = new Set(['pre', 'a', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'summary']);

interface Origin {
  readonly locale: 'en' | 'pl';
  readonly command: string | undefined;
}

function origin(source: string, fileURL: URL | undefined): Origin {
  const marker = /<!--nurt-document:(en|pl):([^>]*?)-->/.exec(source);
  if (marker) {
    const [, locale, id] = marker;
    return { locale: locale as 'en' | 'pl', command: id.startsWith('commands/') ? id.slice('commands/'.length) : undefined };
  }
  return { locale: fileURL && fileURL.pathname.includes('/pages/pl/') ? 'pl' : 'en', command: undefined };
}

export const commandLinks = defineHastPlugin({
  name: 'nurt-command-links',
  element: {
    filter: ['code'],
    visit(node, ctx) {
      const name = ctx.textContent(node).trim();
      if (!commands.has(name)) {
        return;
      }
      const parent = ctx.parent(node);
      const parentName = parent && parent.type === 'element' ? parent.tagName : '';
      if (NEVER.has(parentName) || (!name.includes('.') && (!PROSE.has(parentName) || AMBIGUOUS.has(name)))) {
        return;
      }
      const from = origin(ctx.source, ctx.fileURL);
      if (from.command === name || insideLink(node, (current) => ctx.parent(current))) {
        return;
      }
      const prefix = from.locale === 'pl' ? '/pl' : '';
      return {
        type: 'element',
        tagName: 'a',
        properties: { href: `${base}${prefix}/docs/commands/${name}/`, className: ['command-link'] },
        children: [node],
      };
    },
  },
});
