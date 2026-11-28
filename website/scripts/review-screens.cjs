// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.reviewScreens
// Responsibility:   Saves a page screen by screen at one window size (width or width x height), so that every part of a long page can be reviewed at full size.
// Dependencies:     puppeteer-core supplied by the environment (not a dependency of the package)

const puppeteer = require('puppeteer-core');

const [base, out, size, address, screens = '8'] = process.argv.slice(2);
const [width, fixedHeight] = size.split('x');

(async () => {
  const browser = await puppeteer.launch({ executablePath: process.env.PUPPETEER_EXECUTABLE_PATH, args: ['--no-sandbox', '--disable-gpu'] });
  const page = await browser.newPage();
  const w = Number(width);
  const h = fixedHeight ? Number(fixedHeight) : w < 600 ? 844 : Math.round((w * 9) / 16);
  await page.setViewport({ width: w, height: h });
  await page.evaluateOnNewDocument((theme) => {
    localStorage.setItem('nurt-locale', 'chosen');
    if (theme) localStorage.setItem('nurt-theme', theme);
  }, process.env.REVIEW_THEME || '');
  await page.goto(base + address, { waitUntil: 'networkidle2' });
  const total = await page.evaluate(() => document.documentElement.scrollHeight);
  const name = `${address.replace(/\//g, '_') || 'home'}${process.env.REVIEW_THEME ? `-${process.env.REVIEW_THEME}` : ''}`;
  for (let i = 0; i < Number(screens) && i * h < total; i += 1) {
    await page.evaluate((y) => window.scrollTo(0, y), i * h);
    await new Promise((r) => setTimeout(r, 1400));
    await page.screenshot({ path: `${out}/${name}-${w}-${String(i).padStart(2, '0')}.png` });
  }
  console.log(`${name} ${w} height=${total} screens=${Math.ceil(total / h)}`);
  await browser.close();
})();
