// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.src.scripts.search
// Responsibility:   Search page script: loads the Pagefind module of the built index, runs the query and renders the results as a list of links with excerpts.
// Dependencies:     the Pagefind output of the build, loaded at run time.

interface PagefindResultData {
  url: string;
  excerpt: string;
  meta: { title?: string };
}

interface PagefindModule {
  search(query: string): Promise<{ results: { data(): Promise<PagefindResultData> }[] }>;
  options(settings: Record<string, unknown>): Promise<void>;
}

const root = document.getElementById('search');

function element<K extends keyof HTMLElementTagNameMap>(tag: K, className?: string, text?: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) {
    node.className = className;
  }
  if (text !== undefined) {
    node.textContent = text;
  }
  return node;
}

async function start(host: HTMLElement): Promise<void> {
  const base = host.dataset.index ?? '';
  const limit = Number(host.dataset.results ?? 8);
  const minimum = Number(host.dataset.min ?? 2);
  const form = element('div', 'search__form');
  const input = element('input', 'search__input');
  input.type = 'search';
  input.placeholder = host.dataset.placeholder ?? '';
  input.setAttribute('aria-label', host.dataset.label ?? 'Search');
  input.autocomplete = 'off';
  const status = element('p', 'search__status');
  status.setAttribute('role', 'status');
  const list = element('ol', 'search__results');
  form.append(input);
  host.append(form, status, list);

  let pagefind: PagefindModule | undefined;
  try {
    pagefind = (await import(/* @vite-ignore */ `${base}pagefind.js`)) as PagefindModule;
    await pagefind.options({ baseUrl: base.replace(/pagefind\/$/, '') });
  } catch (error) {
    status.textContent = host.dataset.unavailable ?? '';
    input.disabled = true;
    return;
  }

  let timer: number | undefined;

  async function run(): Promise<void> {
    const query = input.value.trim();
    list.replaceChildren();
    if (query.length < minimum || !pagefind) {
      status.textContent = '';
      return;
    }
    const found = await pagefind.search(query);
    const items = await Promise.all(found.results.slice(0, limit).map((result) => result.data()));
    if (items.length === 0) {
      status.textContent = `${host.dataset.empty ?? ''} "${query}"`;
      return;
    }
    status.textContent = `${found.results.length}`;
    for (const item of items) {
      const entry = element('li', 'search__result');
      const link = element('a', 'search__link', item.meta.title ?? item.url);
      link.href = item.url;
      const excerpt = element('p', 'search__excerpt');
      excerpt.innerHTML = item.excerpt;
      entry.append(link, excerpt);
      list.append(entry);
    }
  }

  input.addEventListener('input', () => {
    window.clearTimeout(timer);
    timer = window.setTimeout(run, 160);
  });
  const initial = new URLSearchParams(window.location.search).get('q');
  if (initial) {
    input.value = initial;
    void run();
  }
  input.focus();
}

if (root) {
  void start(root);
}
