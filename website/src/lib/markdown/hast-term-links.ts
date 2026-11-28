// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.markdown.hast-term-links
// Responsibility:   Hast plugin of the Sätteri Markdown processor that links the first mention of a term of the product in a document to the page that describes it, in the language of the document. Only running text is linked: a paragraph, a list item or a definition; text in a link or a linked card, in code and in a heading stays as it is, and a page does not link to itself.
// Dependencies:     satteri, website.config.site, website.src.lib.markdown.hast-command-links (document marker)

import { defineHastPlugin } from 'satteri';
import { site } from '../../../config/site';
import { insideLink } from './hast-command-links';

type Locale = 'en' | 'pl';

interface Term {
  /** The address of the page that describes the term, without the language prefix. */
  readonly path: string;
  /** The forms of the term as they stand in text, per language; Polish lists its cases. */
  readonly forms: Record<Locale, readonly string[]>;
}

const terms: readonly Term[] = [
  { path: '/product/effect-reports/', forms: { en: ['effect reports', 'effect report', 'report of its effects'], pl: ['raport skutków', 'raportu skutków', 'raportem skutków', 'raporcie skutków', 'raporty skutków', 'raportów skutków'] } },
  { path: '/product/journal-and-undo/', forms: { en: ['journal'], pl: ['dziennikiem', 'dziennika', 'dzienniku', 'dziennik'] } },
  { path: '/product/commands/', forms: { en: ['command catalog'], pl: ['katalogiem poleceń', 'katalogu poleceń', 'katalog poleceń'] } },
  { path: '/product/ai-agents/', forms: { en: ['Model Context Protocol'], pl: ['Model Context Protocol'] } },
  { path: '/product/migration/', forms: { en: ['nurt migrate'], pl: ['nurt migrate'] } },
  { path: '/docs/specification/', forms: { en: ['specification'], pl: ['specyfikacją', 'specyfikację', 'specyfikacji', 'specyfikacja'] } },
  { path: '/docs/cookbook/', forms: { en: ['cookbook'], pl: ['książki kucharskiej', 'książce kucharskiej', 'książka kucharska', 'książkę kucharską'] } },
  { path: '/docs/guide/installation/', forms: { en: ['setup wizard'], pl: ['kreator instalacji', 'kreatora instalacji'] } },
  { path: '/architecture/protocols/', forms: { en: ['language server'], pl: ['serwera języka', 'serwer języka'] } },
  { path: '/architecture/c-interface/', forms: { en: ['C interface'], pl: ['interfejsu C', 'interfejs C'] } },
  { path: '/architecture/crates/', forms: { en: ['workspace'], pl: ['przestrzeni roboczej', 'przestrzeń roboczą', 'przestrzeń robocza'] } },
  { path: '/ai/benchmark/', forms: { en: ['benchmark of tasks', 'nurt-bench'], pl: ['benchmark zadań', 'benchmarku zadań', 'nurt-bench'] } },
  { path: '/ai/training-packages/', forms: { en: ['training packages'], pl: ['paczki treningowe', 'paczek treningowych', 'pakiety treningowe'] } },
  { path: '/ai/model-labs/', forms: { en: ['tool definition'], pl: ['definicję narzędzia', 'definicji narzędzia', 'definicja narzędzia'] } },
  { path: '/quality/conformance/', forms: { en: ['conformance suite'], pl: ['zestawu zgodności', 'zestaw zgodności'] } },
  { path: '/quality/supply-chain/', forms: { en: ['dependency policy'], pl: ['polityki zależności', 'politykę zależności', 'polityka zależności'] } },
  { path: '/quality/testing/', forms: { en: ['clean-system scenarios', 'end-to-end scenarios'], pl: ['scenariusze na czystym systemie', 'scenariuszy end-to-end'] } },
  { path: '/project/versioning/', forms: { en: ['Semantic Versioning'], pl: ['Semantic Versioning'] } },
  { path: '/project/license/', forms: { en: ['Apache License'], pl: ['Apache License'] } },
];

const base = site.base === '/' ? '' : site.base.replace(/\/$/, '');
const PROSE = new Set(['p', 'li', 'dd']);

function escape(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/** One expression per language that finds any form as a whole word; the longer forms stand first. */
const patterns: Record<Locale, RegExp> = {
  en: new RegExp(`(?<![\\p{L}\\p{N}-])(${terms.flatMap((term) => term.forms.en).sort((a, b) => b.length - a.length).map(escape).join('|')})(?![\\p{L}\\p{N}-])`, 'giu'),
  pl: new RegExp(`(?<![\\p{L}\\p{N}-])(${terms.flatMap((term) => term.forms.pl).sort((a, b) => b.length - a.length).map(escape).join('|')})(?![\\p{L}\\p{N}-])`, 'giu'),
};

function termOf(form: string, locale: Locale): Term | undefined {
  const wanted = form.toLowerCase();
  return terms.find((term) => term.forms[locale].some((candidate) => candidate.toLowerCase() === wanted));
}

interface Origin {
  readonly locale: Locale;
  readonly path: string;
}

function origin(source: string, fileURL: URL | undefined): Origin {
  const marker = /<!--nurt-document:(en|pl):([^>]*?)-->/.exec(source);
  if (marker) {
    const id = marker[2].replace(/\/index$/, '');
    return { locale: marker[1] as Locale, path: `/docs/${id}/` };
  }
  const page = fileURL ? /\/pages\/(en|pl)\/(.+)\.mdx?$/.exec(fileURL.pathname) : null;
  return page ? { locale: page[1] as Locale, path: `/${page[2]}/` } : { locale: 'en', path: '' };
}

export const termLinks = defineHastPlugin({
  name: 'nurt-term-links',
  text(node, ctx) {
    const parent = ctx.parent(node);
    if (!parent || parent.type !== 'element' || !PROSE.has(parent.tagName) || insideLink(node, (current) => ctx.parent(current))) {
      return;
    }
    const from = origin(ctx.source, ctx.fileURL);
    const data = ctx.data as { nurtLinkedTerms?: Set<string> };
    const linked = (data.nurtLinkedTerms ??= new Set<string>());
    const pattern = patterns[from.locale];
    pattern.lastIndex = 0;
    for (let match = pattern.exec(node.value); match; match = pattern.exec(node.value)) {
      const term = termOf(match[1], from.locale);
      const rest = node.value.slice(match.index + match[1].length);
      // A term that closes its piece of text names what follows it (a file, a value), and a journal of changes is the changelog, not the journal of the product.
      if (!term || linked.has(term.path) || term.path === from.path || from.path.startsWith(term.path) || rest.trim() === '' || /^ (zmian|of changes)/.test(rest)) {
        continue;
      }
      linked.add(term.path);
      const prefix = from.locale === 'pl' ? '/pl' : '';
      return {
        type: 'element',
        tagName: 'span',
        properties: {},
        children: [
          { type: 'text', value: node.value.slice(0, match.index) },
          { type: 'element', tagName: 'a', properties: { href: `${base}${prefix}${term.path}`, className: ['term-link'] }, children: [{ type: 'text', value: match[1] }] },
          { type: 'text', value: rest },
        ],
      };
    }
    return;
  },
});
