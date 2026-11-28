// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.theme-toggle
// Responsibility:   Switches between the dark and the light theme, stores the choice and keeps the state of the toggle buttons in sync.
// Dependencies:     none.

const STORAGE_KEY = 'nurt-theme';

function currentMode(): 'dark' | 'light' {
  const explicit = document.documentElement.getAttribute('data-theme');
  if (explicit === 'dark' || explicit === 'light') {
    return explicit;
  }
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
}

function reflect(): void {
  const mode = currentMode();
  document.querySelectorAll<HTMLButtonElement>('[data-theme-toggle]').forEach((button) => {
    button.setAttribute('aria-pressed', mode === 'light' ? 'true' : 'false');
    const label = mode === 'light' ? button.dataset.labelDark : button.dataset.labelLight;
    if (label) {
      button.setAttribute('aria-label', label);
      button.setAttribute('title', label);
    }
  });
}

function toggle(): void {
  const next = currentMode() === 'dark' ? 'light' : 'dark';
  document.documentElement.setAttribute('data-theme', next);
  try {
    window.localStorage.setItem(STORAGE_KEY, next);
  } catch (error) {
    document.documentElement.setAttribute('data-theme', next);
  }
  reflect();
}

document.querySelectorAll<HTMLButtonElement>('[data-theme-toggle]').forEach((button) => button.addEventListener('click', toggle));
window.matchMedia('(prefers-color-scheme: light)').addEventListener('change', reflect);
reflect();
