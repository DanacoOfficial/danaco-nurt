// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.angleBrackets
// Responsibility:   Keeps the angle brackets of an imported documentation file as text. The documentation writes types and placeholders in angle brackets (list<text>, map<any>, <base64>) and uses no HTML; without this a browser reads such a name as an unknown element and the name disappears from the page.
// Dependencies:     none

const FENCE = /^\s*(```|~~~)/;
const CODE_SPAN_OR_TAG = /(`+)(?:(?!\n\n)[\s\S])*?\1|<(?=\/?[A-Za-z][A-Za-z0-9-]*(?:[\s/>]|$))/gm;

/** Replaces the opening angle bracket of everything that Markdown would read as an HTML tag by its character reference, outside fenced code and code spans. */
export function literalAngleBrackets(markdown: string): string {
  const blocks: string[] = [];
  let prose: string[] = [];
  let fence: string | null = null;
  const flush = (): void => {
    if (prose.length > 0) {
      blocks.push(prose.join('\n').replace(CODE_SPAN_OR_TAG, (match) => (match === '<' ? '&lt;' : match)));
      prose = [];
    }
  };
  for (const line of markdown.split('\n')) {
    const marker = FENCE.exec(line)?.[1] ?? null;
    if (fence === null && marker !== null) {
      flush();
      fence = marker;
      blocks.push(line);
    } else if (fence !== null) {
      blocks.push(line);
      if (marker === fence) fence = null;
    } else {
      prose.push(line);
    }
  }
  flush();
  return blocks.join('\n');
}
