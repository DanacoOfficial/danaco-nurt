// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.localeInit
// Responsibility:   Runs in the head of an English page that has a Polish version: on the first visit of a browser set to Polish it opens the Polish page once and remembers that the choice was made, so the language switch stays in control afterwards.
// Dependencies:     none

(() => {
  try {
    const key = 'nurt-locale';
    const target = document.documentElement.dataset.plHref;
    if (!target || localStorage.getItem(key)) {
      return;
    }
    localStorage.setItem(key, 'chosen');
    if (/^pl\b/i.test(navigator.language || '')) {
      location.replace(target + location.search + location.hash);
    }
  } catch {
    // Storage is unavailable in some private modes; the page then stays in English.
  }
})();
