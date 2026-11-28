// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.lib.docs.tableCode
// Responsibility:   Keeps a vertical bar written inside a code span of a table row as part of the code. Markdown divides a table row into cells before it reads code spans, so an unescaped bar inside code (`char|block`) cuts the cell and the rest of the row is lost.
// Dependencies:     none

const FENCE = /^\s*(```|~~~)/;
const CODE_SPAN = /(`+)(.*?)\1/g;
const BARE_BAR = /(?<!\\)\|/g;

/** Escapes the vertical bars inside the code spans of table rows, outside fenced code. */
export function tableCodeBars(markdown: string): string {
  let fence: string | null = null;
  return markdown
    .split('\n')
    .map((line) => {
      const marker = FENCE.exec(line)?.[1] ?? null;
      if (fence !== null) {
        if (marker === fence) fence = null;
        return line;
      }
      if (marker !== null) {
        fence = marker;
        return line;
      }
      if (!/^\s*\|/.test(line)) {
        return line;
      }
      return line.replace(CODE_SPAN, (span, ticks: string, code: string) => `${ticks}${code.replace(BARE_BAR, '\\|')}${ticks}`);
    })
    .join('\n');
}
