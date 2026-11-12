import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test, expect, SITE_DIR } from '../lib/support.ts';

// 404.html: GitHub Pages serves it at any depth for any missing address, so it must stand alone.

const ORIGIN = 'https://samer2373.github.io/one-mk/';

test('the 404 page is self-contained: no relative links, styles and the way home are absolute or inline', async () => {
  const html = readFileSync(join(SITE_DIR, '404.html'), 'utf8');
  const refs = [...html.matchAll(/(?:src|href)="([^"]+)"/g)].map((m) => m[1]!).filter((u) => !u.startsWith('#'));
  const relative = refs.filter((u) => !/^(https?:|mailto:|data:)/.test(u));
  expect(relative, 'relative links break at depth').toEqual([]);
  expect(html).toContain('noindex');
  expect(html).not.toMatch(/<script[^>]+src=/);
  // Every absolute link stays on the project's own addresses.
  for (const u of refs) expect(u.startsWith(ORIGIN) || u.startsWith('https://github.com/samer2373/one-mk/'), u).toBe(true);
});

test('404.html shows its message, one h1, a way home, and links that work', async ({ app, browser, screen, guard }) => {
  guard.mirror(ORIGIN);
  await app.open('/404.html');
  await expect(screen.getByRole('heading', { name: 'Nothing past this edge.' })).toBeVisible();
  const r = await browser.evaluate(() => ({
    h1: document.querySelectorAll('h1').length, title: document.title, lang: document.documentElement.lang,
    home: Array.from(document.querySelectorAll('a')).map((a) => ({ t: (a.getAttribute('aria-label') ?? a.textContent ?? '').trim(), h: a.getAttribute('href') })),
    main: document.querySelectorAll('main').length,
  }));
  expect(r.h1).toBe(1);
  expect(r.title).toBe('Page not found · oneMK');
  expect(r.lang).toBe('en');
  expect(r.main).toBe(1);
  expect(r.home.map((h) => h.h)).toEqual([ORIGIN, ORIGIN, 'https://github.com/samer2373/one-mk/releases/latest']);
  expect(r.home.find((h) => h.t === 'Back to oneMK')).toBeTruthy();
  // Its fonts come from the (mirrored) site address, and loaded.
  expect(guard.requests.some((u) => u.endsWith('Inter-Regular.woff2') || u.endsWith('Inter-SemiBold.woff2'))).toBe(true);
  await guard.check();
});

for (const scheme of ['light', 'dark'] as const) {
  test(`404.html in ${scheme}: readable, no sideways scroll, focus ring on its links`, async ({ app, browser, guard }) => {
    guard.mirror(ORIGIN);
    await browser.emulateMedia({ colorScheme: scheme });
    await app.open('/404.html');
    const r = await browser.evaluate(() => {
      const de = document.documentElement;
      const cs = getComputedStyle(document.body);
      return { theme: de.dataset.theme, over: de.scrollWidth - de.clientWidth, bg: cs.backgroundColor, ink: cs.color };
    });
    expect(r.theme).toBe(scheme);
    expect(r.over).toBeLessThanOrEqual(0);
    expect(r.bg).toBe(scheme === 'dark' ? 'rgb(17, 18, 20)' : 'rgb(244, 243, 240)');
    await browser.keyboard.press('Tab');
    const ring = await browser.evaluate(() => { const e = document.activeElement as HTMLElement; const s = getComputedStyle(e); return { tag: e.tagName, ring: s.outlineStyle !== 'none' && parseFloat(s.outlineWidth) > 0 }; });
    expect(ring).toEqual({ tag: 'A', ring: true });
    await guard.check();
  });
}

test('404.html with reduced motion does not animate the pointer', async ({ app, browser, guard }) => {
  guard.mirror(ORIGIN);
  await browser.emulateMedia({ reducedMotion: 'reduce' });
  await app.open('/404.html');
  expect(await browser.evaluate<string>("getComputedStyle(document.querySelector('.edge b')).animationName")).toBe('none');
  await guard.check();
});

test('a missing address on the server is a real 404 status', async ({ app }) => {
  const r = await fetch(`${app.baseUrl}/no-such-page`);
  expect(r.status).toBe(404);
});
