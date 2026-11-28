// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           website.scripts.reviewContrast
// Responsibility:   Measures the contrast of every visible text of the given pages against its ground, in the light and in the dark theme, and lists the texts below the level AA of WCAG (4.5 : 1, and 3 : 1 for large text). A ground that is an image or a gradient is not measured; the solid colours behind the text are.
// Dependencies:     puppeteer-core supplied by the environment (not a dependency of the package)

const puppeteer = require('puppeteer-core');

const [base, ...addresses] = process.argv.slice(2);

/** Runs in the page: returns the kinds of text below the required contrast, each with its lowest ratio and a sample. */
function measure() {
  const probe = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
  // Any colour syntax of the style sheet is read by painting one pixel, so mixed and wide-gamut colours are measured as the browser shows them.
  const parse = (value) => {
    probe.clearRect(0, 0, 1, 1);
    probe.fillStyle = '#000';
    probe.fillStyle = value;
    probe.fillRect(0, 0, 1, 1);
    const [red, green, blue, alpha] = probe.getImageData(0, 0, 1, 1).data;
    return { rgb: [red, green, blue], alpha: alpha / 255 };
  };
  const luminance = (rgb) => {
    const [red, green, blue] = rgb.map((part) => {
      const share = part / 255;
      return share <= 0.03928 ? share / 12.92 : ((share + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * red + 0.7152 * green + 0.0722 * blue;
  };
  const over = (top, alpha, bottom) => top.map((part, index) => part * alpha + bottom[index] * (1 - alpha));
  const ground = (element) => {
    const layers = [];
    for (let node = element; node; node = node.parentElement) {
      const colour = parse(getComputedStyle(node).backgroundColor);
      if (colour.alpha > 0) {
        layers.push(colour);
        if (colour.alpha >= 1) break;
      }
    }
    const canvas = parse(getComputedStyle(document.documentElement).colorScheme.includes('dark') ? '#000' : '#fff').rgb;
    return layers.reverse().reduce((below, layer) => over(layer.rgb, layer.alpha, below), canvas);
  };
  const found = {};
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    if (!node.data.trim()) continue;
    const element = node.parentElement;
    const style = getComputedStyle(element);
    if (style.visibility === 'hidden' || element.getClientRects().length === 0) continue;
    let opacity = 1;
    for (let part = element; part; part = part.parentElement) opacity *= Number(getComputedStyle(part).opacity);
    // Text that is not shown at all (a closed panel, a layer waiting for its entrance) is not read by anyone.
    if (opacity < 0.05) continue;
    const below = ground(element);
    const colour = parse(style.color);
    // A dimmed element shows its text through: the opacity counts as part of the colour.
    const text = over(colour.rgb, colour.alpha * opacity, below);
    const [lighter, darker] = [luminance(text), luminance(below)].sort((first, second) => second - first);
    const ratio = (lighter + 0.05) / (darker + 0.05);
    const size = parseFloat(style.fontSize);
    const large = size >= 24 || (size >= 18.66 && Number(style.fontWeight) >= 700);
    if (ratio >= (large ? 3 : 4.5)) continue;
    const name = typeof element.className === 'string' && element.className ? `${element.tagName.toLowerCase()}.${element.className.split(' ')[0]}` : `${element.tagName.toLowerCase()} in ${element.parentElement.tagName.toLowerCase()}.${String(element.parentElement.className).split(' ')[0]}`;
    const entry = found[name] ?? { count: 0, ratio, sample: node.data.trim().slice(0, 32), size };
    entry.count += 1;
    entry.ratio = Math.min(entry.ratio, ratio);
    found[name] = entry;
  }
  return found;
}

(async () => {
  const browser = await puppeteer.launch({ executablePath: process.env.PUPPETEER_EXECUTABLE_PATH, args: ['--no-sandbox', '--disable-gpu'] });
  let kinds = 0;
  for (const theme of ['light', 'dark']) {
    const page = await browser.newPage();
    await page.setViewport({ width: 1440, height: 900 });
    // Entrances fade the text in; with reduced motion the text has its final colour when it is measured.
    await page.emulateMediaFeatures([{ name: 'prefers-reduced-motion', value: 'reduce' }]);
    await page.evaluateOnNewDocument((chosen) => {
      localStorage.setItem('nurt-locale', 'chosen');
      localStorage.setItem('nurt-theme', chosen);
    }, theme);
    for (const address of addresses) {
      await page.goto(base + address, { waitUntil: 'networkidle2' });
      const found = await page.evaluate(measure);
      for (const [name, entry] of Object.entries(found)) {
        kinds += 1;
        console.log(`${theme} ${address}: ${name} ${entry.ratio.toFixed(2)} : 1 at ${entry.size}px, ${entry.count} text(s), for example "${entry.sample}"`);
      }
    }
    await page.close();
  }
  await browser.close();
  console.log(`pages=${addresses.length} themes=2 kinds of text below the level=${kinds}`);
})();
