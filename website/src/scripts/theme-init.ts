// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.theme-init
// Responsibility:   Applies the stored colour theme before the first paint so that the page does not flash. The file is inlined into the head by the base layout.
// Dependencies:     none.

(function applyStoredTheme() {
  try {
    var stored = window.localStorage.getItem('nurt-theme');
    if (stored === 'dark' || stored === 'light') {
      document.documentElement.setAttribute('data-theme', stored);
    }
  } catch (error) {
    document.documentElement.removeAttribute('data-theme');
  }
})();
