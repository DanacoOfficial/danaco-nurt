// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.search-shortcut
// Responsibility:   Opens the search page when the reader presses the key / outside a field, as the key shown in the search box of the header says.
// Dependencies:     none.

document.addEventListener('keydown', (event) => {
  if (event.key !== '/' || event.ctrlKey || event.metaKey || event.altKey) {
    return;
  }
  const target = event.target as HTMLElement | null;
  if (target && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))) {
    return;
  }
  const link = document.querySelector<HTMLAnchorElement>('[data-search-shortcut]');
  if (link) {
    event.preventDefault();
    window.location.assign(link.href);
  }
});
