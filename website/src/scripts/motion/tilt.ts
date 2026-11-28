// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.motion.tilt
// Responsibility:   Tilts a link card or a statistic card in perspective towards the pointer and moves a highlight under it. Works only for a fine pointer that can hover; touch screens keep flat cards.
// Dependencies:     none.

const maximumAngle = 6;

export function startTilt(): void {
  if (!window.matchMedia('(hover: hover) and (pointer: fine)').matches) {
    return;
  }
  const cards = Array.from(document.querySelectorAll<HTMLElement>('.card--link, .stat'));
  for (const card of cards) {
    let frame = 0;
    card.addEventListener('pointermove', (event) => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        const box = card.getBoundingClientRect();
        const x = (event.clientX - box.left) / box.width;
        const y = (event.clientY - box.top) / box.height;
        card.style.setProperty('--tilt-x', `${((0.5 - y) * maximumAngle).toFixed(2)}deg`);
        card.style.setProperty('--tilt-y', `${((x - 0.5) * maximumAngle).toFixed(2)}deg`);
        card.style.setProperty('--glow-x', `${(x * 100).toFixed(1)}%`);
        card.style.setProperty('--glow-y', `${(y * 100).toFixed(1)}%`);
        card.classList.add('is-tilted');
      });
    });
    card.addEventListener('pointerleave', () => {
      cancelAnimationFrame(frame);
      card.classList.remove('is-tilted');
      card.style.removeProperty('--tilt-x');
      card.style.removeProperty('--tilt-y');
    });
  }
}
