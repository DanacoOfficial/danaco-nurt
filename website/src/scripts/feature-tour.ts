// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.featureTour
// Responsibility:   Behaviour of the feature tour: choosing a tab by pointer or keyboard shows its panel, and while the tour is in view and not in use the tabs advance on their own every few seconds. With reduced motion the tabs never advance on their own.
// Dependencies:     none

const interval = 7000;

function startTour(root: HTMLElement): void {
  const tabs = Array.from(root.querySelectorAll<HTMLButtonElement>('[data-tour-tab]'));
  const panels = Array.from(root.querySelectorAll<HTMLElement>('[data-tour-panel]'));
  if (tabs.length === 0 || tabs.length !== panels.length) {
    return;
  }
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  // On a narrow screen the panels differ in height more, so the tabs do not advance on their own there.
  const wide = window.matchMedia('(min-width: 64rem)');
  let current = 0;
  let timer = 0;
  let visible = false;
  let engaged = false;

  const select = (index: number, focus = false): void => {
    current = (index + tabs.length) % tabs.length;
    tabs.forEach((tab, position) => {
      const active = position === current;
      tab.setAttribute('aria-selected', active ? 'true' : 'false');
      tab.tabIndex = active ? 0 : -1;
      tab.classList.remove('is-running');
    });
    panels.forEach((panel, position) => panel.toggleAttribute('data-active', position === current));
    if (focus) {
      tabs[current].focus();
    }
    schedule();
  };

  const schedule = (): void => {
    window.clearTimeout(timer);
    tabs.forEach((tab) => tab.classList.remove('is-running'));
    if (reduced || !visible || engaged || !wide.matches) {
      return;
    }
    // The class starts the progress line of the active tab; forcing a layout restarts the line on every turn.
    void tabs[current].offsetWidth;
    tabs[current].classList.add('is-running');
    timer = window.setTimeout(() => select(current + 1), interval);
  };

  root.setAttribute('data-tour-ready', '');
  root.style.setProperty('--tour-interval', `${interval}ms`);
  tabs.forEach((tab, index) => {
    tab.addEventListener('click', () => select(index));
    tab.addEventListener('keydown', (event) => {
      if (event.key === 'ArrowDown' || event.key === 'ArrowRight') {
        event.preventDefault();
        select(current + 1, true);
      } else if (event.key === 'ArrowUp' || event.key === 'ArrowLeft') {
        event.preventDefault();
        select(current - 1, true);
      } else if (event.key === 'Home') {
        event.preventDefault();
        select(0, true);
      } else if (event.key === 'End') {
        event.preventDefault();
        select(tabs.length - 1, true);
      }
    });
  });
  const engage = (value: boolean) => (): void => {
    engaged = value;
    schedule();
  };
  root.addEventListener('pointerenter', engage(true));
  root.addEventListener('pointerleave', engage(false));
  root.addEventListener('focusin', engage(true));
  root.addEventListener('focusout', engage(false));
  new IntersectionObserver(
    (entries) => {
      visible = entries.some((entry) => entry.isIntersecting);
      schedule();
    },
    { threshold: 0.35 },
  ).observe(root);
  wide.addEventListener('change', schedule);
}

document.querySelectorAll<HTMLElement>('[data-tour]').forEach(startTour);
