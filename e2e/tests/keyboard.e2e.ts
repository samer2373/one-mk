import { test, expect } from '../lib/support.ts';

// Keyboard use: nothing is a trap, everything that takes focus shows it, the order makes sense.

interface Stop { label: string; tag: string; top: number; left: number; bottom: number; right: number; ring: boolean; visible: boolean; inBar: boolean; id: string }

/** The page scrolls smoothly to a newly focused control, so `here` first waits until it has stopped. */
async function here(browser: import('@e2e-dev/web').Browser): Promise<Stop | null> {
  return browser.evaluate(async () => {
    await new Promise((r) => setTimeout(r, 120));
    await new Promise<void>((done) => {
      let last = -1, same = 0, n = 0;
      const tick = () => { same = window.scrollY === last ? same + 1 : 0; last = window.scrollY; if (same >= 3 || ++n > 90) done(); else requestAnimationFrame(tick); };
      requestAnimationFrame(tick);
    });
    const e = document.activeElement as HTMLElement | null;
    if (!e || e === document.body || e === document.documentElement) return null;
    const r = e.getBoundingClientRect(), cs = getComputedStyle(e);
    const ring = (cs.outlineStyle !== 'none' && parseFloat(cs.outlineWidth) > 0) || cs.boxShadow !== 'none';
    const label = (e.getAttribute('aria-label') ?? e.textContent ?? e.getAttribute('href') ?? '').replace(/\s+/g, ' ').trim().slice(0, 50);
    const matches = e.matches(':focus-visible');
    return {
      label: label || e.tagName, tag: e.tagName.toLowerCase(), top: Math.round(r.top + window.scrollY), left: Math.round(r.left), bottom: Math.round(r.bottom), right: Math.round(r.right),
      ring: ring && matches, visible: r.width > 0 && r.height > 0 && cs.visibility !== 'hidden', inBar: !!e.closest('header.bar'), id: e.id,
    };
  });
}

test('Tab walks the whole page front to back: no trap, every stop is visible and shows a focus ring', async ({ app, browser, guard }) => {
  await app.open('/');
  const expected = await browser.evaluate<number>(`(function(){
    var sel = 'a[href], button, summary, input, select, textarea, [tabindex]:not([tabindex="-1"])';
    return Array.from(document.querySelectorAll(sel)).filter(function(e){ return !e.closest('details:not([open])') || e.tagName === 'SUMMARY' && e.parentElement.matches('details') ; }).filter(function(e){ var r = e.getBoundingClientRect(); return (r.width > 0 && r.height > 0) && !e.disabled; }).length;
  })()`);
  expect(expected).toBeGreaterThan(60);
  const stops: Stop[] = [];
  const seen = new Set<string>();
  let left = false;
  for (let i = 0; i < expected + 12; i++) {
    await browser.keyboard.press('Tab');
    const s = await here(browser);
    if (!s) { left = true; break; }
    const key = `${s.tag}|${s.label}|${s.top}|${s.left}`;
    if (seen.has(key)) { left = true; break; } // came back round to the start
    seen.add(key);
    stops.push(s);
  }
  expect(left, `Tab never left the page in ${expected + 12} presses (a keyboard trap?)`).toBe(true);
  // At least every visible control got a stop (tab strips and the like use one stop per group).
  expect(stops.length).toBeGreaterThan(expected - 20);
  expect(stops[0]!.label).toBe('Skip to content');
  expect(stops.at(-1)!.label).toMatch(/Inter|JetBrains|Unbounded|three\.js|Apache|MIT|CC BY|creativecommons|Okopchi|Logitech/i);
  const noRing = stops.filter((s) => !s.ring).map((s) => `${s.tag} "${s.label}"`);
  expect(noRing, 'stops without a visible focus ring').toEqual([]);
  const hidden = stops.filter((s) => !s.visible).map((s) => `${s.tag} "${s.label}"`);
  expect(hidden, 'focus landed on something you cannot see').toEqual([]);
  await guard.check();
});

test('while tabbing, the focused control is never hidden behind the top bar or off the sides', async ({ app, browser, guard }) => {
  await app.open('/');
  const vw = await browser.evaluate<number>('document.documentElement.clientWidth');
  const bar = await browser.evaluate<number>("Math.round(document.querySelector('header.bar').getBoundingClientRect().bottom)");
  const problems: string[] = [];
  for (let i = 0; i < 160; i++) {
    await browser.keyboard.press('Tab');
    const s = await browser.evaluate(async () => {
      await new Promise((r) => setTimeout(r, 120));
      await new Promise<void>((done) => {
        let last = -1, same = 0, n = 0;
        const tick = () => { same = window.scrollY === last ? same + 1 : 0; last = window.scrollY; if (same >= 3 || ++n > 90) done(); else requestAnimationFrame(tick); };
        requestAnimationFrame(tick);
      });
      const e = document.activeElement as HTMLElement | null;
      if (!e || e === document.body) return null;
      const r = e.getBoundingClientRect();
      return { label: (e.getAttribute('aria-label') ?? e.textContent ?? '').replace(/\s+/g, ' ').trim().slice(0, 40), top: r.top, bottom: r.bottom, left: r.left, right: r.right, vh: window.innerHeight, inBar: !!e.closest('header.bar'), skip: e.classList.contains('skip'), summary: e.tagName === 'SUMMARY' };
    });
    if (!s) break;
    if (!s.inBar && !s.skip && s.top < bar - 1) problems.push(`under the bar: ${s.label} (top ${Math.round(s.top)} < ${bar})`);
    // A tall control may run past the fold, but its start must be on screen, and it must fit sideways.
    if (s.top > s.vh - 8 || s.right > vw + 1 || s.left < -1) problems.push(`off screen: ${s.label} (${Math.round(s.left)}..${Math.round(s.right)} x ${Math.round(s.top)}..${Math.round(s.bottom)})`);
  }
  expect(problems).toEqual([]);
  await guard.check();
});

test('tab stops go through the sections in page order, and the download cards in the order they appear on screen', async ({ app, browser, guard }) => {
  await app.open('/');
  const stops: { label: string; section: number; card: string; cardTop: number; cardLeft: number }[] = [];
  for (let i = 0; i < 170; i++) {
    await browser.keyboard.press('Tab');
    const s = await browser.evaluate(async () => {
      await new Promise((r) => setTimeout(r, 120));
      await new Promise<void>((done) => {
        let last = -1, same = 0, n = 0;
        const tick = () => { same = window.scrollY === last ? same + 1 : 0; last = window.scrollY; if (same >= 3 || ++n > 90) done(); else requestAnimationFrame(tick); };
        requestAnimationFrame(tick);
      });
      const e = document.activeElement as HTMLElement | null;
      if (!e || e === document.body) return null;
      const blocks = Array.from(document.querySelectorAll('a.skip, header.bar, main > section, footer'));
      const card = e.closest('[data-panel]') as HTMLElement | null;
      const cr = card?.getBoundingClientRect();
      return { label: (e.getAttribute('aria-label') ?? e.textContent ?? '').replace(/\s+/g, ' ').trim().slice(0, 40), section: blocks.findIndex((b) => b.contains(e)),
        card: card?.dataset.panel ?? '', cardTop: cr ? Math.round(cr.top + window.scrollY) : 0, cardLeft: cr ? Math.round(cr.left) : 0 };
    });
    if (!s) break;
    stops.push(s);
  }
  const back: string[] = [];
  stops.forEach((s, i) => { if (i && s.section < stops[i - 1]!.section) back.push(`"${s.label}" is in an earlier section than "${stops[i - 1]!.label}"`); });
  expect(back).toEqual([]);
  expect(stops.every((s) => s.section >= 0)).toBe(true);
  // The cards, in the order Tab meets them, are in reading order on screen: top to bottom, then left to right.
  const cards: { card: string; top: number; left: number }[] = [];
  for (const s of stops) if (s.card && cards.at(-1)?.card !== s.card) cards.push({ card: s.card, top: s.cardTop, left: s.cardLeft });
  expect(cards.length).toBe(3);
  const visual = [...cards].sort((a, b) => (Math.abs(a.top - b.top) > 20 ? a.top - b.top : a.left - b.left)).map((c) => c.card);
  expect(cards.map((c) => c.card), 'Tab order of the download cards differs from what is on screen').toEqual(visual);
  await guard.check();
});

test('nothing hidden from assistive technology can still take focus, and nothing sets a positive tabindex', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const sel = 'a[href], button, summary, input, select, textarea, [tabindex]';
    const hiddenFocusable = Array.from(document.querySelectorAll('[aria-hidden="true"]')).flatMap((h) =>
      Array.from(h.querySelectorAll(sel)).filter((e) => (e as HTMLElement).tabIndex >= 0).map((e) => `${e.tagName} in aria-hidden ${h.className || h.tagName}`));
    const positive = Array.from(document.querySelectorAll('[tabindex]')).filter((e) => (e as HTMLElement).tabIndex > 0).map((e) => e.outerHTML.slice(0, 60));
    const inClosed = Array.from(document.querySelectorAll('details:not([open]) > :not(summary) a, details:not([open]) > :not(summary) button')).filter((e) => (e as HTMLElement).offsetParent !== null).map((e) => (e.textContent ?? '').trim().slice(0, 30));
    const hiddenPanels = Array.from(document.querySelectorAll('[role="tabpanel"][hidden] a, [role="tabpanel"][hidden] button')).length;
    return { hiddenFocusable, positive, inClosed, hiddenPanels };
  });
  expect(r.hiddenFocusable).toEqual([]);
  expect(r.positive).toEqual([]);
  expect(r.hiddenPanels).toBe(0);
  await guard.check();
});

test('Shift+Tab goes back one stop at a time', async ({ app, browser, guard }) => {
  await app.open('/');
  const labels: string[] = [];
  for (let i = 0; i < 4; i++) { await browser.keyboard.press('Tab'); labels.push((await here(browser))!.label); }
  for (let i = 2; i >= 0; i--) {
    await browser.keyboard.down('Shift');
    await browser.keyboard.press('Tab');
    await browser.keyboard.up('Shift');
    expect((await here(browser))!.label).toBe(labels[i]);
  }
  await guard.check();
});

test('links and buttons answer Enter, and buttons answer Space', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const toggle = screen.getByRole('button', { name: /Switch to (dark|light) theme/ });
  await toggle.focus();
  const before = await browser.evaluate<string>('document.documentElement.dataset.theme');
  await browser.keyboard.press('Space');
  expect(await browser.evaluate<string>('document.documentElement.dataset.theme')).not.toBe(before);
  const heroLink = screen.getByRole('link', { name: 'Other downloads' });
  await heroLink.focus();
  await browser.keyboard.press('Enter');
  await expect(browser).toHaveURL(/#download$/);
  await guard.check();
});
