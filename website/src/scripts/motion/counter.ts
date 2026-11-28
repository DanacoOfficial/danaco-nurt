// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.motion.counter
// Responsibility:   Counts the numbers of the statistic cards up from zero when a card enters the view. Every integer in the text is counted, its grouping separator is kept, and the final text is always the value written by the page.
// Dependencies:     none.

const duration = 1400;
const numberPattern = /\d[\d,\u00a0\u202f ]*\d|\d/g;

interface CountedNumber {
  value: number;
  separator: string;
}

function easeOutCubic(progress: number): number {
  return 1 - Math.pow(1 - progress, 3);
}

function render(template: string, numbers: CountedNumber[], progress: number): string {
  let index = 0;
  return template.replace(numberPattern, () => {
    const number = numbers[index];
    index += 1;
    const current = String(Math.round(number.value * progress));
    return number.separator ? current.replace(/\B(?=(\d{3})+(?!\d))/g, number.separator) : current;
  });
}

function animate(element: HTMLElement): void {
  const template = element.textContent ?? '';
  const matches = template.match(numberPattern);
  if (!matches || !/^\d/.test(template.trim())) {
    return;
  }
  const numbers: CountedNumber[] = matches.map((text) => ({
    value: Number(text.replace(/\D/g, '')),
    separator: (text.match(/\D/) ?? [''])[0],
  }));
  const start = performance.now();

  const step = (now: number): void => {
    const progress = Math.min((now - start) / duration, 1);
    element.textContent = progress < 1 ? render(template, numbers, easeOutCubic(progress)) : template;
    if (progress < 1) {
      requestAnimationFrame(step);
    }
  };
  requestAnimationFrame(step);
}

export function startCounters(): void {
  const values = Array.from(document.querySelectorAll<HTMLElement>('.stat__value'));
  if (values.length === 0 || !('IntersectionObserver' in window)) {
    return;
  }
  const observer = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting) {
          animate(entry.target as HTMLElement);
          observer.unobserve(entry.target);
        }
      }
    },
    { threshold: 0.6 },
  );
  values.forEach((value) => observer.observe(value));
}
