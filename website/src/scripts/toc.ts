// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.toc
// Responsibility:   Marks the table of contents entry of the heading that is in view while the page is read.
// Dependencies:     none.

const links = Array.from(document.querySelectorAll<HTMLAnchorElement>('.toc a[href^="#"]'));
const targets = links
  .map((link) => document.getElementById(decodeURIComponent(link.hash.slice(1))))
  .filter((element): element is HTMLElement => element !== null);

function mark(id: string | null): void {
  for (const link of links) {
    if (id !== null && decodeURIComponent(link.hash.slice(1)) === id) {
      link.setAttribute('aria-current', 'true');
    } else {
      link.removeAttribute('aria-current');
    }
  }
}

if (targets.length > 0 && 'IntersectionObserver' in window) {
  const observer = new IntersectionObserver(
    (entries) => {
      const visible = entries.filter((entry) => entry.isIntersecting).sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
      if (visible.length > 0) {
        mark(visible[0].target.id);
      }
    },
    { rootMargin: '-72px 0px -70% 0px', threshold: [0, 1] },
  );
  targets.forEach((target) => observer.observe(target));
}
