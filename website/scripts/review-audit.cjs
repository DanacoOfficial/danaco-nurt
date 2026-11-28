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
    for (const cell of document.querySelectorAll('.prose .table--whole-words :is(th, td), .prose .table--whole-text :is(th, td)')) {
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
  await browser.close();
  console.log(report.join('\n'));
  console.log(`pages=${list.length} widths=${widths.join(',')} with issues=${report.length}`);
})();
