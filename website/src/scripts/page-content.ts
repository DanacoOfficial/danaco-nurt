// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.pageContent
// Responsibility:   Keeps programs, command names, terminals and code out of the automatic translation of the browser, so a translated page still shows commands that run; gives every table cell the heading of its column, which the narrow layout of tables shows above the cell.
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
