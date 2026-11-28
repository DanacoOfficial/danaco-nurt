// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.motion.reveal
// Responsibility:   Entrance of elements as they scroll into view: each target receives the class is-visible once, and siblings of one group receive a growing delay so that a row of cards enters in sequence.
// Dependencies:     website.src.scripts.motion.targets

import { revealSelectors } from './targets';

const staggerStep = 70;
const staggerLimit = 6;

export function startReveal(): void {
  const targets = Array.from(document.querySelectorAll<HTMLElement>(revealSelectors.join(',')));
  if (targets.length === 0) {
    return;
  }
  const targetSet = new Set<Element>(targets);

  for (const target of targets) {
    const parent = target.parentElement;
    if (!parent) {
      continue;
    }
    const siblings = Array.from(parent.children).filter((child) => targetSet.has(child));
    const index = Math.min(siblings.indexOf(target), staggerLimit);
    target.style.setProperty('--reveal-delay', `${Math.max(index, 0) * staggerStep}ms`);
  }

  if (!('IntersectionObserver' in window)) {
    targets.forEach((target) => target.classList.add('is-visible'));
    return;
  }

  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          entry.target.classList.add('is-visible');
          observer.unobserve(entry.target);
        }
      }
    },
    { rootMargin: '0px 0px -6% 0px', threshold: 0.06 },
  );

  targets.forEach((target) => observer.observe(target));
}
