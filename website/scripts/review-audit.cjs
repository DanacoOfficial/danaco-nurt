// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.reviewAudit
// Responsibility:   Opens the pages of the built site in a headless browser at the given widths and reports layout defects: horizontal scroll, elements outside the window, inner horizontal scroll bars, empty bands beside the text, a navigation running under the header tools, repeated identifiers, script errors and English documentation text on Polish pages.
// Dependencies:     puppeteer-core supplied by the environment (not a dependency of the package), a local server of the directory dist

const puppeteer = require('puppeteer-core');
const fs = require('fs');
const path = require('path');

const base = process.argv[2] || 'http://127.0.0.1:4400';
const widths = (process.argv[3] || '1440,390').split(',').map(Number);
const only = process.argv[4] ? new RegExp(process.argv[4]) : null;
const dist = path.join(__dirname, '..', 'dist');

/** Collects the addresses of all pages from the files index.html of the directory dist. */
function pages(directory, prefix = '/') {
  const list = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && !['_assets', 'pagefind', 'downloads', 'schema', 'brand', 'social'].includes(entry.name)) list.push(...pages(path.join(directory, entry.name), `${prefix}${entry.name}/`));
    else if (entry.name === 'index.html') list.push(prefix);
  }
  return list;
}

/** Checks one loaded page and returns the defects found. */
function inspect() {
  const issues = [];
  const root = document.documentElement;
  const width = window.innerWidth;
  if (root.scrollWidth > width + 1) issues.push(`horizontal scroll ${root.scrollWidth}>${width}`);
  for (const element of document.querySelectorAll('main *')) {
    const box = element.getBoundingClientRect();
    if (!box.width || !box.height) continue;
    if (box.right > width + 1 && !element.closest('pre, .table-scroll, [data-decor], svg, canvas, .hero__scene, .mark-scene')) {
      issues.push(`outside the window: ${element.tagName.toLowerCase()}.${String(element.className).split(' ')[0]}`);
      break;
    }
  }
  for (const element of document.querySelectorAll('main pre, main .session__screen, main .terminal, main figure, main .diagram')) {
    if (element.scrollWidth > element.clientWidth + 2 && width >= 1000 && getComputedStyle(element).overflowX !== 'visible') {
      issues.push(`inner horizontal scroll: ${element.tagName.toLowerCase()}.${String(element.className).split(' ')[0]} ${element.scrollWidth}>${element.clientWidth}`);
      break;
    }
  }
  const main = document.querySelector('.docs-page__main, .page-shell__main');
  const prose = main ? main.querySelector('.prose') : null;
  if (main && prose && width >= 1200) {
    const empty = main.getBoundingClientRect().width - prose.getBoundingClientRect().width;
    if (empty > 160) issues.push(`empty band beside the text ${Math.round(empty)}px`);
  }
  const navigation = document.querySelector('.site-header__nav');
  const tools = document.querySelector('.site-header__tools');
  if (navigation && tools && getComputedStyle(navigation).display !== 'none') {
    const last = navigation.querySelector('li:last-child');
    if (last && last.getBoundingClientRect().right > tools.getBoundingClientRect().left + 1) issues.push('navigation under the header tools');
  }
  if (!document.querySelector('main h1')) issues.push('no h1');
  const ids = Array.from(document.querySelectorAll('[id]')).map((element) => element.id);
  const repeated = ids.find((id, index) => ids.indexOf(id) !== index);
  if (repeated) issues.push(`repeated id ${repeated}`);
  const labels = [document.title, ...Array.from(document.querySelectorAll('h1, h2, h3, h4, nav a')).map((element) => element.textContent)];
  const marker = labels.find((label) => /\{#[^}]*\}/.test(label));
  if (marker) issues.push(`anchor marker in a label: ${marker.trim().slice(0, 60)}`);
  if (width > 740) {
    const range = document.createRange();
    let cut = '';
    for (const cell of document.querySelectorAll('.prose :is(.table--whole-code, .table--whole-words, .table--whole-text) :is(th, td)')) {
      const codeMayBreak = Boolean(cell.closest('.table--whole-text'));
      const walker = document.createTreeWalker(cell, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node && !cut; node = walker.nextNode()) {
        if (codeMayBreak && node.parentElement.closest('code')) continue;
        for (const word of node.data.matchAll(/[\p{L}\p{N}_.]{2,24}/gu)) {
          range.setStart(node, word.index);
          range.setEnd(node, word.index + word[0].length);
          const tops = new Set(Array.from(range.getClientRects()).map((box) => Math.round(box.top)));
          if (tops.size > 1) {
            cut = word[0];
            break;
          }
        }
      }
      if (cut) break;
    }
    if (cut) issues.push(`word cut in a table cell: ${cut}`);
  }
  if (root.lang.startsWith('pl') && location.pathname.includes('/docs/')) {
    const text = Array.from(document.querySelectorAll('.prose h2, .prose h3')).map((heading) => heading.textContent.trim());
    if (text.some((label) => /^(Synopsis|Description|Parameters|Examples|Errors|Related|Effect|Input and output)$/.test(label))) issues.push('English documentation headings on a Polish page');
  }
  return issues;
}

(async () => {
  const browser = await puppeteer.launch({ executablePath: process.env.PUPPETEER_EXECUTABLE_PATH, args: ['--no-sandbox', '--disable-gpu'] });
  const page = await browser.newPage();
  await page.evaluateOnNewDocument(() => localStorage.setItem('nurt-locale', 'chosen'));
  const errors = [];
  page.on('pageerror', (error) => errors.push(error.message));
  const list = pages(dist).filter((address) => !only || only.test(address));
  const report = [];
  for (const width of widths) {
    await page.setViewport({ width, height: 900 });
    for (const address of list) {
      errors.length = 0;
      const response = await page.goto(base + address, { waitUntil: 'load' });
      const issues = await page.evaluate(inspect);
      if (response && response.status() >= 400) issues.push(`status ${response.status()}`);
      issues.push(...errors.map((message) => `script error ${message}`));
      if (issues.length) report.push(`${width} ${address}: ${issues.join('; ')}`);
    }
  }
  // A translating browser lengthens the labels of the navigation; the header must then change to its compact form and never let the labels run under the tools.
  await page.setViewport({ width: 1280, height: 900 });
  await page.goto(`${base}/`, { waitUntil: 'load' });
  await page.evaluate(() => {
    document.querySelectorAll('.site-header__nav a').forEach((link) => {
      link.textContent = `${link.textContent} ${link.textContent} ${link.textContent}`;
    });
  });
  await page.waitForFunction(() => document.querySelector('.site-header').classList.contains('is-compact'), { timeout: 3000 }).catch(() => undefined);
  const translated = await page.evaluate(() => {
    const header = document.querySelector('.site-header');
    const navigation = document.querySelector('.site-header__nav');
    const tools = document.querySelector('.site-header__tools');
    const shown = getComputedStyle(navigation).display !== 'none';
    const overlap = shown && navigation.querySelector('li:last-child').getBoundingClientRect().right > tools.getBoundingClientRect().left + 1;
    return { compact: header.classList.contains('is-compact'), overlap, button: getComputedStyle(document.querySelector('.site-header__menu-button')).display };
  });
  if (!translated.compact || translated.overlap || translated.button === 'none') report.push(`1280 /: header with long labels: ${JSON.stringify(translated)}`);
  // The menu button must open the menu and the menu must stay open: in the compact header of a wide window and on a narrow window.
  const menuStaysOpen = async () => {
    await page.click('[data-menu-toggle]');
    await new Promise((resolve) => setTimeout(resolve, 700));
    return page.evaluate(() => {
      const panel = document.getElementById('site-menu');
      return document.querySelector('[data-menu-toggle]').getAttribute('aria-expanded') === 'true' && panel.getBoundingClientRect().height > 100;
    });
  };
  if (translated.button !== 'none' && !(await menuStaysOpen())) report.push('1280 /: the menu of the compact header does not stay open');
  for (const narrow of [390, 768]) {
    await page.setViewport({ width: narrow, height: 900 });
    await page.goto(`${base}/`, { waitUntil: 'load' });
    if (!(await menuStaysOpen())) report.push(`${narrow} /: the menu does not stay open`);
  }
  // A missing Polish address must show the Polish page for a missing address with the address that was asked for, and its menu must work. A server that does not answer a missing address with the page of the site (a plain file server) is skipped.
  const missing = '/pl/no-such-page-of-the-site/';
  await page.goto(base + missing, { waitUntil: 'networkidle2' });
  if (await page.evaluate(() => Boolean(document.querySelector('.not-found')))) {
    const state = await page.evaluate(() => ({ lang: document.documentElement.lang, address: location.pathname, header: document.querySelector('.site-header__panel a').getAttribute('href') }));
    if (!state.lang.startsWith('pl') || state.address !== missing || !state.header.startsWith('/pl/')) report.push(`768 ${missing}: the page for a missing Polish address: ${JSON.stringify(state)}`);
    if (!(await menuStaysOpen())) report.push(`768 ${missing}: the menu does not stay open`);
  }
  // On the pages for a missing address the language switch must lead to the home page of the other language.
  for (const [address, home] of [['/404.html', '/pl/'], ['/pl/404/', '/']]) {
    await page.goto(base + address, { waitUntil: 'load' });
    const other = await page.evaluate(() => document.querySelector('.language-switch a')?.getAttribute('href') ?? null);
    if (other !== home) report.push(`768 ${address}: the language switch leads to ${other} instead of ${home}`);
  }
  // The parts of a page that answer a click must answer it: the navigation of the documentation on a narrow window, the colour theme, the install tabs, the feature tour and the search.
  const pause = () => new Promise((resolve) => setTimeout(resolve, 500));
  await page.goto(`${base}/pl/docs/guide/`, { waitUntil: 'load' });
  await page.click('[data-docs-menu]');
  await pause();
  if (!(await page.evaluate(() => document.getElementById('docs-menu').hasAttribute('data-open') && document.getElementById('docs-menu').getBoundingClientRect().height > 100))) report.push('768 /pl/docs/guide/: the navigation of the documentation does not open');
  await page.setViewport({ width: 1440, height: 900 });
  await page.goto(`${base}/`, { waitUntil: 'load' });
  const themeBefore = await page.evaluate(() => document.documentElement.getAttribute('data-theme'));
  await page.click('[data-theme-toggle]');
  await pause();
  const themeAfter = await page.evaluate(() => document.documentElement.getAttribute('data-theme'));
  if (themeAfter === themeBefore) report.push('1440 /: the theme toggle does not change the theme');
  await page.goto(`${base}/product/overview/`, { waitUntil: 'load' });
  if ((await page.evaluate(() => document.documentElement.getAttribute('data-theme'))) !== themeAfter) report.push('1440 /product/overview/: the chosen theme is not kept on the next page');
  await page.goto(`${base}/`, { waitUntil: 'load' });
  await page.click('.install-tabs__labels label:nth-of-type(2)');
  await pause();
  if (!(await page.evaluate(() => getComputedStyle(document.querySelector('.install-tabs__panel--windows')).display !== 'none' && getComputedStyle(document.querySelector('.install-tabs__panel--unix')).display === 'none'))) report.push('1440 /: the install tabs do not switch');
  await (await page.$$('[data-tour-tab]'))[2].click();
  const tourChanged = await page
    .waitForFunction(() => document.querySelectorAll('[data-tour-panel]')[2].hasAttribute('data-active') && document.querySelectorAll('[data-tour-tab]')[2].getAttribute('aria-selected') === 'true', { timeout: 3000 })
    .then(() => true)
    .catch(() => false);
  if (!tourChanged) report.push('1440 /: the feature tour does not show the chosen panel');
  for (const [address, query] of [['/search/', 'replace'], ['/pl/search/', 'dziennik']]) {
    await page.goto(base + address, { waitUntil: 'load' });
    await page.waitForSelector('.search__input:not([disabled])', { timeout: 5000 }).catch(() => undefined);
    await page.type('.search__input', query).catch(() => undefined);
    await page.waitForSelector('.search__result', { timeout: 6000 }).catch(() => undefined);
    const found = await page.evaluate(() => document.querySelectorAll('.search__result').length);
    if (found === 0) report.push(`1440 ${address}: the search finds nothing for ${query}`);
  }
  await page.goto(`${base}/docs/commands/copy/`, { waitUntil: 'load' });
  await (await page.$('.prose pre .copy-button')).click();
  const copied = await page
    .waitForFunction(() => document.querySelector('.prose pre .copy-button').textContent === document.documentElement.dataset.labelCopied, { timeout: 3000 })
    .then(() => true)
    .catch(() => false);
  if (!copied) report.push('1440 /docs/commands/copy/: the copy button does not report a copy');
  // A browser set to Polish that opens the English home page for the first time must land on the Polish home page, and must stay on the English page when it returns to it.
  const context = await browser.createBrowserContext();
  const visitor = await context.newPage();
  await visitor.evaluateOnNewDocument(() => Object.defineProperty(navigator, 'language', { get: () => 'pl-PL' }));
  await visitor.goto(`${base}/`, { waitUntil: 'load' });
  await visitor.waitForFunction(() => location.pathname.startsWith('/pl/'), { timeout: 3000 }).catch(() => undefined);
  const first = new URL(visitor.url()).pathname;
  await visitor.goto(`${base}/`, { waitUntil: 'load' });
  const second = new URL(visitor.url()).pathname;
  if (first !== '/pl/' || second !== '/') report.push(`1280 /: Polish browser: first visit ${first}, return ${second}`);
  await context.close();
  // A browser set to Polish that starts on a Polish page and chooses English with the language switch must stay on the English page.
  const secondContext = await browser.createBrowserContext();
  const chooser = await secondContext.newPage();
  await chooser.setViewport({ width: 1440, height: 900 });
  await chooser.evaluateOnNewDocument(() => Object.defineProperty(navigator, 'language', { get: () => 'pl-PL' }));
  await chooser.goto(`${base}/pl/product/overview/`, { waitUntil: 'load' });
  await Promise.all([chooser.waitForNavigation({ waitUntil: 'load' }), chooser.click('.language-switch a')]);
  await chooser.waitForFunction(() => location.pathname.startsWith('/pl/'), { timeout: 1500 }).catch(() => undefined);
  const chosen = new URL(chooser.url()).pathname;
  if (chosen !== '/product/overview/') report.push(`1440 /pl/product/overview/: the language switch to English ends on ${chosen}`);
  await secondContext.close();
  await browser.close();
  console.log(report.join('\n'));
  console.log(`pages=${list.length} widths=${widths.join(',')} with issues=${report.length}`);
})();
