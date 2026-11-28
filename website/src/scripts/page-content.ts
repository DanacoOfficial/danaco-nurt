// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.pageContent
// Responsibility:   Keeps programs, command names, terminals and code out of the automatic translation of the browser, so a translated page still shows commands that run; gives every table cell the heading of its column, which the narrow layout of tables shows above the cell. It also chooses for each table how its cells may break: code on one line and whole words while the table fits its column, then whole words with code that wraps, then breaks inside code, and anywhere as the last step.
// Dependencies:     none

const selector = 'pre, code, kbd, samp, .terminal, .session, .install-tabs, .command-catalog code';

/** Marks every element with code on the page as not to be translated. */
export function protectCode(root: ParentNode = document): void {
  root.querySelectorAll<HTMLElement>(selector).forEach((element) => {
    element.setAttribute('translate', 'no');
    element.classList.add('notranslate');
  });
}

/** Copies the heading of each column into the cells below it as the attribute data-label. */
export function labelTableCells(root: ParentNode = document): void {
  root.querySelectorAll<HTMLTableElement>('.prose table').forEach((table) => {
    const headings = Array.from(table.querySelectorAll<HTMLTableCellElement>('thead th')).map((cell) => cell.textContent?.trim() ?? '');
    table.querySelectorAll<HTMLTableRowElement>('tbody tr').forEach((row) => {
      Array.from(row.cells).forEach((cell, index) => {
        if (headings[index]) {
          cell.dataset.label = headings[index];
        }
      });
    });
  });
}

/** The marks of a table from the strictest breaking to the loosest: code on one line, whole words, whole words in text only. A table with no mark breaks anywhere. */
const tableMarks = ['table--whole-code', 'table--whole-words', 'table--whole-text'] as const;

/** Sets the breaking of one table: the strictest step with which the table still fits its container. Code on one line is kept only when it does not make the table taller, because a long piece of code would otherwise take the width from the text beside it. */
function fitTable(table: HTMLTableElement): void {
  const container = table.parentElement;
  if (!container) {
    return;
  }
  const [wholeCode, ...others] = tableMarks;
  const measure = (mark: string): { fits: boolean; height: number } => {
    table.classList.remove(...tableMarks);
    table.classList.add(mark);
    const box = table.getBoundingClientRect();
    return { fits: box.width <= container.clientWidth + 1, height: box.height };
  };
  const strict = measure(wholeCode);
  for (const mark of others) {
    const loose = measure(mark);
    if (!loose.fits) {
      continue;
    }
    if (strict.fits && strict.height <= loose.height) {
      measure(wholeCode);
    }
    return;
  }
  table.classList.remove(...tableMarks);
}

/** Keeps words whole in the cells of every table that has room for them, and repeats the choice when the window changes its width. */
export function fitTables(root: ParentNode = document): void {
  const tables = Array.from(root.querySelectorAll<HTMLTableElement>('.prose table'));
  if (tables.length === 0) {
    return;
  }
  const fitAll = (): void => tables.forEach(fitTable);
  fitAll();
  let width = window.innerWidth;
  let frame = 0;
  window.addEventListener('resize', () => {
    if (window.innerWidth === width) {
      return;
    }
    width = window.innerWidth;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(fitAll);
  });
  document.fonts?.ready.then(fitAll).catch(() => undefined);
}
