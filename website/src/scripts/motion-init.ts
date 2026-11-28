// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.motion-init
// Responsibility:   Marks the document for motion before the first paint when the reader has not asked for reduced motion, so that elements waiting for their entrance never flash. The class is withdrawn when the motion module has not started within three seconds, which keeps the page readable if a script fails. The file is inlined into the head by the base layout.
// Dependencies:     none.

(function markMotion() {
  var root = document.documentElement;
  var reduced = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  if (reduced) {
    return;
  }
  root.classList.add('motion');
  window.setTimeout(function withdrawWhenIdle() {
    if (!root.hasAttribute('data-motion-ready')) {
      root.classList.remove('motion');
    }
  }, 3000);
})();
