// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.menu
// Responsibility:   Opens and closes the navigation panel on narrow screens: the toggle state, the Escape key and the click outside the panel. Fits the header to the width of its navigation, and records a choice made with the language switch, so the first visit of a Polish browser to an English page does not undo it.
// Dependencies:     none.

const button = document.querySelector<HTMLButtonElement>('[data-menu-toggle]');
const panel = document.getElementById('site-menu');

function setOpen(open: boolean): void {
  if (!button || !panel) {
    return;
  }
  button.setAttribute('aria-expanded', open ? 'true' : 'false');
  const label = open ? button.dataset.labelClose : button.dataset.labelOpen;
  if (label) {
    button.setAttribute('aria-label', label);
  }
  panel.toggleAttribute('data-open', open);
  document.body.toggleAttribute('data-menu-open', open);
}

if (button && panel) {
  button.addEventListener('click', () => setOpen(button.getAttribute('aria-expanded') !== 'true'));
  document.addEventListener('keydown', (event) => {
    if (event.key === 'Escape' && button.getAttribute('aria-expanded') === 'true') {
      setOpen(false);
      button.focus();
    }
  });
  panel.addEventListener('click', (event) => {
    if ((event.target as HTMLElement).closest('a')) {
      setOpen(false);
    }
  });
  window.matchMedia('(min-width: 60rem)').addEventListener('change', (event) => {
    if (event.matches) {
      setOpen(false);
    }
  });
}

/** Switches the header to its compact form whenever the navigation is wider than the room left beside the tools. */
function fitHeader(): void {
  const header = document.querySelector<HTMLElement>('.site-header');
  const navigation = header?.querySelector<HTMLElement>('.site-header__nav');
  if (!header || !navigation) {
    return;
  }
  const measure = (): void => {
    header.classList.remove('is-compact');
    // On a narrow window the style sheet hides the navigation and shows the menu button; there is nothing to measure, and the open menu must stay open.
    if (getComputedStyle(navigation).display === 'none') {
      return;
    }
    if (navigation.scrollWidth > navigation.clientWidth + 1) {
      header.classList.add('is-compact');
    } else {
      setOpen(false);
    }
  };
  measure();
  new ResizeObserver(measure).observe(header);
  new MutationObserver(measure).observe(navigation, { childList: true, characterData: true, subtree: true });
  document.fonts?.ready.then(measure);
}

fitHeader();

/** A click on the language switch is a choice of language: it sets the mark that the first-visit script reads, so that script leaves the chosen page as it is. */
document.querySelectorAll<HTMLAnchorElement>('.language-switch a').forEach((link) => {
  link.addEventListener('click', () => {
    try {
      localStorage.setItem('nurt-locale', 'chosen');
    } catch {
      // Storage is unavailable in some private modes; the link works without the mark.
    }
  });
});
