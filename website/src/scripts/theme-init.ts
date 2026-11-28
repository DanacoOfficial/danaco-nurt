// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.theme-init
// Responsibility:   Applies the stored colour theme before the first paint so that the page does not flash; the identity is dark, so the dark theme is the default. The file is inlined into the head by the base layout.
// Dependencies:     none.

(function applyStoredTheme() {
  try {
    var stored = window.localStorage.getItem('nurt-theme');
    document.documentElement.setAttribute('data-theme', stored === 'light' ? 'light' : 'dark');
  } catch (error) {
    document.documentElement.setAttribute('data-theme', 'dark');
  }
})();
