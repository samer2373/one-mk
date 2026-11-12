// Opens site/index.html and site/404.html straight from disk (file://) in
// Chromium, Firefox and WebKit. The e2e runner only opens http(s) addresses,
// so this is a small script on the same Playwright the runner uses.
//   npm run test:file
import { chromium, firefox, webkit } from 'playwright-core';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join } from 'node:path';

const site = join(dirname(fileURLToPath(import.meta.url)), '..', 'site');
const engines = { chromium, firefox, webkit };
const sizes = [['desktop', { width: 1280, height: 720 }], ['phone', { width: 390, height: 844 }], ['small', { width: 320, height: 568 }]];
const failures = [];
let checks = 0;
const ok = (cond, what) => { checks++; if (!cond) failures.push(what); };

for (const [name, type] of Object.entries(engines)) {
  const browser = await type.launch();
  for (const [sizeName, viewport] of sizes) {
    const tag = `${name}/${sizeName}`;
    const ctx = await browser.newContext({ viewport, colorScheme: 'dark', reducedMotion: 'reduce' });
    const page = await ctx.newPage();
    page.setDefaultTimeout(120_000); page.setDefaultNavigationTimeout(120_000);
    const problems = [];
    const foreign = [];
    page.on('pageerror', (e) => problems.push(`exception: ${e.message}`));
    page.on('console', (m) => { if (['error', 'warning'].includes(m.type())) problems.push(`console.${m.type()}: ${m.text()}`); });
    page.on('requestfailed', (r) => problems.push(`request failed: ${r.url()} ${r.failure()?.errorText}`));
    page.on('request', (r) => { if (!/^(file|data|blob|about):/.test(r.url())) foreign.push(r.url()); });

    await page.goto(pathToFileURL(join(site, 'index.html')).href);
    await page.waitForLoadState('load');
    ok((await page.locator('h1').innerText()).includes('One mouse'), `${tag}: heading`);
    ok((await page.evaluate(() => document.documentElement.dataset.theme)) === 'dark', `${tag}: follows prefers-color-scheme`);
    // The fonts come from the inlined stylesheet, which only file:// uses.
    ok((await page.locator('link[href="css/fonts-file.css"]').count()) === 1, `${tag}: file:// fonts stylesheet`);
    ok((await page.evaluate(() => document.fonts.check('16px Inter'))), `${tag}: Inter loaded`);
    // Download chooser ran, and the primary button points at a release file or the section.
    await page.waitForFunction(() => document.documentElement.dataset.detectDone === '1');
    const href = await page.locator('a[data-download]').first().getAttribute('href');
    ok(/^https:\/\/github\.com\/samer2373\/one-mk\/releases\/latest\/download\/one-mk-|^#download$|\/releases\/latest$/.test(href ?? ''), `${tag}: download button ${href}`);
    // Theme toggle.
    await page.locator('[data-theme-toggle]').click();
    ok((await page.evaluate(() => document.documentElement.dataset.theme)) === 'light', `${tag}: toggle`);
    // Tabs.
    await page.locator('#t-health').scrollIntoViewIfNeeded();
    await page.locator('#t-health').click();
    ok(await page.locator('#p-health').isVisible(), `${tag}: tab panel`);
    // Posters are real images.
    ok(await page.evaluate(() => Array.from(document.querySelectorAll('.stage__poster img')).some((i) => i.offsetWidth > 0 && i.complete && i.naturalWidth > 0)), `${tag}: poster`);
    ok(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth), `${tag}: no sideways scroll`);
    ok(foreign.length === 0, `${tag}: requests to other places: ${foreign.join(' ')}`);
    ok(problems.length === 0, `${tag}: ${problems.join(' | ')}`);

    // The 404 page opens from disk too (its links are absolute by design).
    await page.route(/^https?:/, (r) => r.abort());
    await page.goto(pathToFileURL(join(site, '404.html')).href);
    ok((await page.locator('h1').innerText()) === 'Nothing past this edge.', `${tag}: 404 heading`);
    await ctx.close();
  }
  await browser.close();
}

if (failures.length) {
  console.error(`FAIL: ${failures.length} of ${checks} checks\n  ` + failures.join('\n  '));
  process.exit(1);
}
console.log(`PASS: ${checks} checks across ${Object.keys(engines).length} browsers x ${sizes.length} sizes from file://`);
