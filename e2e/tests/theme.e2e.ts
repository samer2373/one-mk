import { test, expect } from '../lib/support.ts';

// Light, dark, the system preference, the toggle and what it remembers.

/** The colours settle after the 450 ms cross-fade. */
const fade = async (browser: import('@e2e-dev/web').Browser) => {
  await expect.poll(() => browser.evaluate<boolean>("document.documentElement.classList.contains('theme-anim')"), { timeout: 3000 }).toBe(false);
  await browser.evaluate('new Promise(function (r) { setTimeout(r, 400); })'); // the last colour transition
};

const state = (browser: import('@e2e-dev/web').Browser) => browser.evaluate(() => {
  const metas = Array.from(document.querySelectorAll('meta[name="theme-color"]')).map((m) => (m as HTMLMetaElement).content);
  const cs = getComputedStyle(document.body);
  const btn = document.querySelector('[data-theme-toggle]') as HTMLElement;
  let stored: string | null = null;
  try { stored = localStorage.getItem('omk-theme'); } catch { stored = 'blocked'; }
  const vis = (el: Element | null) => !!el && getComputedStyle(el).display !== 'none';
  return {
    theme: document.documentElement.dataset.theme ?? '',
    scheme: getComputedStyle(document.documentElement).colorScheme,
    bg: cs.backgroundColor, ink: cs.color,
    label: btn.getAttribute('aria-label') ?? '',
    metas, stored,
    sun: vis(btn.querySelector('.t-dark')), moon: vis(btn.querySelector('.t-light')),
  };
});

test('the page starts in the theme the system asks for, light or dark, before it is painted', async ({ app, browser, guard }) => {
  for (const scheme of ['dark', 'light'] as const) {
    await browser.emulateMedia({ colorScheme: scheme });
    await app.open('/');
    const s = await state(browser);
    expect(s.theme).toBe(scheme);
    expect(s.scheme).toContain(scheme);
    expect(s.bg).toBe(scheme === 'dark' ? 'rgb(17, 18, 20)' : 'rgb(244, 243, 240)');
    expect(s.label).toBe(scheme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme');
    expect(s.metas.length).toBe(2);
    for (const m of s.metas) expect(m).toBe(scheme === 'dark' ? '#111214' : '#F4F3F0');
  }
  await guard.check();
});

test('the theme script runs in the head before the stylesheet, so there is no flash of the wrong theme', async ({ app }) => {
  const html = await (await fetch(`${app.baseUrl}/`)).text();
  const head = html.slice(0, html.indexOf('</head>'));
  const script = head.indexOf('omk-theme');
  const sheet = head.indexOf('rel="stylesheet" href="css/site.css"');
  expect(script).toBeGreaterThan(0);
  expect(sheet).toBeGreaterThan(script);
  // ... and it is not deferred or async.
  const tag = head.slice(head.lastIndexOf('<script', script), script);
  expect(tag).not.toMatch(/defer|async|type="module"/);
});

test('the toggle flips the theme both ways, relabels itself, and swaps the icon and the browser colour', async ({ app, browser, screen, guard }) => {
  await browser.emulateMedia({ colorScheme: 'light' });
  await app.open('/');
  const toggle = screen.getByRole('button', { name: /Switch to (dark|light) theme/ });
  const a = await state(browser);
  expect(a.theme).toBe('light');
  expect(a.moon).toBe(true);
  expect(a.sun).toBe(false);
  await toggle.tap();
  await fade(browser);
  const b = await state(browser);
  expect(b).toMatchObject({ theme: 'dark', label: 'Switch to light theme', stored: 'dark', sun: true, moon: false });
  expect(b.bg).not.toBe(a.bg);
  expect(b.ink).not.toBe(a.ink);
  for (const m of b.metas) expect(m).toBe('#111214');
  await expect(screen.getByRole('button', { name: 'Switch to light theme' })).toBeVisible();
  await screen.getByRole('button', { name: 'Switch to light theme' }).tap();
  await fade(browser);
  const c = await state(browser);
  expect(c).toMatchObject({ theme: 'light', label: 'Switch to dark theme', stored: 'light', sun: false, moon: true });
  expect(c.bg).toBe(a.bg);
  for (const m of c.metas) expect(m).toBe('#F4F3F0');
  // The cross-fade class does not stay on the page.
  await expect.poll(() => browser.evaluate<boolean>("document.documentElement.classList.contains('theme-anim')"), { timeout: 3000 }).toBe(false);
  await guard.check();
});

test('the toggle works from the keyboard (Enter and Space) and keeps focus', async ({ app, browser, screen, guard }) => {
  await browser.emulateMedia({ colorScheme: 'light' });
  await app.open('/');
  await screen.getByRole('button', { name: 'Switch to dark theme' }).focus();
  await browser.keyboard.press('Enter');
  expect((await state(browser)).theme).toBe('dark');
  await expect(screen.getByRole('button', { name: 'Switch to light theme' })).toBeFocused();
  await browser.keyboard.press('Space');
  expect((await state(browser)).theme).toBe('light');
  await guard.check();
});

test('a chosen theme is remembered across reloads and beats the system preference', async ({ app, browser, screen, guard }) => {
  await browser.emulateMedia({ colorScheme: 'light' });
  await app.open('/');
  await screen.getByRole('button', { name: 'Switch to dark theme' }).tap();
  await browser.reload();
  expect((await state(browser)).theme).toBe('dark');
  // The system now says light, but the visitor chose dark.
  await browser.emulateMedia({ colorScheme: 'light' });
  await browser.reload();
  expect((await state(browser)).theme).toBe('dark');
  // And the other way round.
  await screen.getByRole('button', { name: 'Switch to light theme' }).tap();
  await browser.emulateMedia({ colorScheme: 'dark' });
  await browser.reload();
  expect((await state(browser)).theme).toBe('light');
  await guard.check();
});

test('without a saved choice the page follows the system when it changes while the page is open', async ({ app, browser, guard }) => {
  await browser.emulateMedia({ colorScheme: 'light' });
  await app.open('/');
  expect((await state(browser)).theme).toBe('light');
  await browser.emulateMedia({ colorScheme: 'dark' });
  await expect.poll(async () => (await state(browser)).theme).toBe('dark');
  await browser.emulateMedia({ colorScheme: 'light' });
  await expect.poll(async () => (await state(browser)).theme).toBe('light');
  // A saved choice stops the following.
  await browser.evaluate("localStorage.setItem('omk-theme', 'light')");
  await browser.emulateMedia({ colorScheme: 'dark' });
  await browser.evaluate('new Promise((r) => setTimeout(r, 600))');
  expect((await state(browser)).theme).toBe('light');
  await guard.check();
});

test('a bad saved value is ignored', async ({ app, browser, guard }) => {
  await browser.emulateMedia({ colorScheme: 'dark' });
  await browser.addInitScript("try { localStorage.setItem('omk-theme', 'purple'); } catch (e) {}");
  await app.open('/');
  expect((await state(browser)).theme).toBe('dark');
  await guard.check();
});

test('storage that throws (private mode) does not break the page or the toggle', async ({ app, browser, screen, guard }) => {
  await browser.emulateMedia({ colorScheme: 'light' });
  await browser.addInitScript(`
    Object.defineProperty(window, 'localStorage', { get: function () { throw new Error('denied'); }, configurable: true });
  `);
  await app.open('/');
  await screen.getByRole('button', { name: 'Switch to dark theme' }).tap();
  expect(await browser.evaluate<string>("document.documentElement.dataset.theme")).toBe('dark');
  await guard.check();
});

test('both themes give readable body text on the page background (WCAG AA)', async ({ app, browser, guard }) => {
  const lum = (c: number[]) => { const f = (v: number) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]!) + 0.7152 * f(c[1]!) + 0.0722 * f(c[2]!); };
  const parse = (s: string) => (s.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
  for (const scheme of ['light', 'dark'] as const) {
    await browser.emulateMedia({ colorScheme: scheme });
    await app.open('/');
    const s = await state(browser);
    const a = lum(parse(s.bg)), b = lum(parse(s.ink));
    const ratio = (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
    expect(ratio, `${scheme} body contrast`).toBeGreaterThan(7);
  }
  await guard.check();
});
