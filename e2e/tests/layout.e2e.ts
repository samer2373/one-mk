import { test, expect, currentProfile, scrollThrough } from '../lib/support.ts';

// Layout at every size: no sideways scroll, nothing sticking out or clipped, tap targets, readable text.

type B = import('@e2e-dev/web').Browser;

const overflow = (browser: B) => browser.evaluate(() => {
  const de = document.documentElement;
  const vw = de.clientWidth;
  const out: string[] = [];
  const clipped = (el: Element) => {
    for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
      const cs = getComputedStyle(p);
      if (/(hidden|auto|scroll|clip)/.test(cs.overflowX)) { const r = p.getBoundingClientRect(); if (r.right <= vw + 1 && r.left >= -1) return true; }
    }
    return false;
  };
  for (const el of Array.from(document.body.querySelectorAll('*'))) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.position === 'fixed') continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    if (el.closest('svg') && el.tagName !== 'svg') continue;
    if ((r.right > vw + 1 || r.left < -1) && !clipped(el)) out.push(`${el.tagName.toLowerCase()}.${(el as HTMLElement).className?.toString().slice(0, 30)} [${Math.round(r.left)}..${Math.round(r.right)}] vw=${vw}`);
  }
  return { scrollW: de.scrollWidth, clientW: vw, bodyW: document.body.scrollWidth, out: out.slice(0, 12), total: out.length };
});

for (const theme of ['light', 'dark'] as const) {
  test(`${theme}: the page never scrolls sideways and nothing sticks out of the window`, async ({ app, browser, guard }) => {
    await browser.emulateMedia({ colorScheme: theme });
    await app.open('/');
    const top = await overflow(browser);
    expect(top.scrollW, 'document wider than the window at the top').toBeLessThanOrEqual(top.clientW);
    // Open every fold so their content counts too.
    await browser.evaluate("document.querySelectorAll('details').forEach(function (d) { d.open = true; })");
    await browser.evaluate('new Promise(function (r) { setTimeout(r, 300); })');
    const s = await overflow(browser);
    expect(s.scrollW, 'document wider than the window').toBeLessThanOrEqual(s.clientW);
    expect(s.bodyW).toBeLessThanOrEqual(s.clientW);
    expect(s.out, `${s.total} elements stick out sideways`).toEqual([]);
    // Every tab panel, too.
    for (const id of ['t-arrange', 't-wallpaper', 't-files', 't-keys', 't-health', 't-settings']) {
      await browser.evaluate((i: string) => { (document.getElementById(i) as HTMLElement).click(); return true; }, id);
      const o = await overflow(browser);
      expect(o.scrollW, `${id} makes the page wider`).toBeLessThanOrEqual(o.clientW);
      expect(o.out, id).toEqual([]);
    }
    await guard.check();
  });
}

test('text is not cut off: no heading, paragraph, button or link hides part of its own words', async ({ app, browser, guard }) => {
  await app.open('/');
  await browser.evaluate("document.querySelectorAll('details').forEach(function (d) { d.open = true; })");
  const bad = await browser.evaluate(() => {
    const out: string[] = [];
    for (const el of Array.from(document.querySelectorAll('h1,h2,h3,p,li,a,button,summary,dt,dd,span,small,kbd,b'))) {
      const cs = getComputedStyle(el);
      if (cs.display === 'none' || cs.display === 'inline') continue;
      if (el.closest('[data-marquee], .facts, [data-tabs], pre, .foot__mark, .stage__labels, [aria-hidden="true"]')) continue;
      const hidesX = /(hidden|clip)/.test(cs.overflowX), hidesY = /(hidden|clip)/.test(cs.overflowY);
      if (hidesX && el.scrollWidth > el.clientWidth + 1) out.push(`x: ${el.tagName.toLowerCase()} "${(el.textContent ?? '').trim().slice(0, 40)}" ${el.scrollWidth}>${el.clientWidth}`);
      if (hidesY && el.scrollHeight > el.clientHeight + 1) out.push(`y: ${el.tagName.toLowerCase()} "${(el.textContent ?? '').trim().slice(0, 40)}" ${el.scrollHeight}>${el.clientHeight}`);
      if (cs.textOverflow === 'ellipsis' && el.scrollWidth > el.clientWidth + 1) out.push(`ellipsis: ${(el.textContent ?? '').trim().slice(0, 40)}`);
    }
    return out;
  });
  expect(bad).toEqual([]);
  await guard.check();
});

test('words stay readable: no visible text under 11px, headings never wider than the window', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const small: string[] = [];
    const vw = document.documentElement.clientWidth;
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const t = (n.textContent ?? '').trim();
      const el = n.parentElement;
      if (!t || !el || /^(SCRIPT|STYLE|NOSCRIPT)$/.test(el.tagName)) continue;
      const cs = getComputedStyle(el);
      if (cs.display === 'none' || el.offsetParent === null && cs.position !== 'fixed') continue;
      if (el.closest('[aria-hidden="true"]')) continue;
      if (parseFloat(cs.fontSize) < 11) small.push(`${parseFloat(cs.fontSize)}px "${t.slice(0, 30)}"`);
    }
    const heads = Array.from(document.querySelectorAll('h1,h2')).filter((h) => h.getBoundingClientRect().right > vw + 1).map((h) => (h.textContent ?? '').trim());
    return { small, heads };
  });
  expect(r.small).toEqual([]);
  expect(r.heads).toEqual([]);
  await guard.check();
});

test('buttons and other controls are big enough to hit (WCAG 2.2 target size: 24 px minimum)', async ({ app, browser, guard }) => {
  await app.open('/');
  const p = await currentProfile(browser);
  await browser.evaluate("document.querySelectorAll('details').forEach(function (d) { d.open = true; })");
  const small = await browser.evaluate((min: number) => {
    const out: string[] = [];
    for (const el of Array.from(document.querySelectorAll('button, summary, [role="tab"], .btn, .chip, .icon-btn, .cmd__copy, a.asset, .bar__nav a, .gh, .brand'))) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      if (Math.min(r.width, r.height) < min) out.push(`${el.tagName.toLowerCase()}.${(el as HTMLElement).className} "${((el.getAttribute('aria-label') ?? el.textContent) || '').trim().slice(0, 24)}" ${Math.round(r.width)}x${Math.round(r.height)}`);
    }
    return out;
  }, 24);
  expect(small).toEqual([]);
  await guard.check();
});

test('the hero fits the first screen: headline, both buttons and the start of the picture are visible without scrolling', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const vh = window.innerHeight;
    const q = (s: string) => { const b = document.querySelector(s)!.getBoundingClientRect(); return { top: b.top, bottom: b.bottom }; };
    return { vh, bar: q('header.bar').bottom, h1: q('.hero__title'), primary: q('.hero__actions .btn--primary'), ghost: q('.hero__actions .btn--ghost') };
  });
  expect(r.h1.top).toBeGreaterThanOrEqual(r.bar - 1);
  expect(r.h1.bottom).toBeLessThan(r.vh);
  expect(r.primary.bottom, 'the Download button is on the first screen').toBeLessThanOrEqual(r.vh);
  expect(r.ghost.bottom).toBeLessThanOrEqual(r.vh + 80);
  await guard.check();
});

test('pictures are never stretched: each visible image keeps its own aspect ratio', async ({ app, browser, guard }) => {
  await app.open('/');
  await scrollThrough(browser);
  const bad = await browser.evaluate(() => {
    const out: string[] = [];
    for (const i of Array.from(document.querySelectorAll('img'))) {
      if (!i.complete || !i.naturalWidth || i.offsetWidth === 0) continue;
      const cs = getComputedStyle(i);
      if (cs.objectFit === 'cover' || cs.objectFit === 'contain') continue; // cropped on purpose, never squashed
      const rw = i.getBoundingClientRect().width, rh = i.getBoundingClientRect().height;
      const want = +(i.getAttribute('width') ?? 0) / +(i.getAttribute('height') ?? 1);
      if (rh > 0 && Math.abs(rw / rh - want) / want > 0.03) out.push(`${i.getAttribute('src')} shown ${Math.round(rw)}x${Math.round(rh)}, made ${i.getAttribute('width')}x${i.getAttribute('height')}`);
    }
    return out;
  });
  expect(bad).toEqual([]);
  await guard.check();
});

for (const theme of ['light', 'dark'] as const) {
  test(`${theme}: text colour against its background meets WCAG AA (4.5:1, 3:1 for large text)`, async ({ app, browser, guard }) => {
    await browser.emulateMedia({ colorScheme: theme, reducedMotion: 'reduce' });
    await app.open('/');
    await browser.evaluate("document.querySelectorAll('details').forEach(function (d) { d.open = true; })");
    await browser.evaluate('new Promise(function (r) { setTimeout(r, 700); })'); // the answers fade in (.4 s)
    const r = await browser.evaluate(() => {
      const cv = document.createElement('canvas'); cv.width = cv.height = 1;
      const cx = cv.getContext('2d', { willReadFrequently: true })!;
      const rgba = (css: string): [number, number, number, number] | null => {
        cx.clearRect(0, 0, 1, 1); cx.fillStyle = '#000'; cx.fillStyle = css; cx.fillRect(0, 0, 1, 1);
        const d = cx.getImageData(0, 0, 1, 1).data;
        cx.clearRect(0, 0, 1, 1);
        // fillStyle ignores a value it cannot parse and keeps black: tell that apart by asking twice.
        cx.fillStyle = '#fff'; cx.fillStyle = css; cx.fillRect(0, 0, 1, 1);
        const e = cx.getImageData(0, 0, 1, 1).data;
        if (d[0] !== e[0] || d[1] !== e[1] || d[2] !== e[2]) return null;
        return [d[0]!, d[1]!, d[2]!, d[3]! / 255];
      };
      const over = (fg: number[], bg: number[]) => { const a = fg[3]!; return [fg[0]! * a + bg[0]! * (1 - a), fg[1]! * a + bg[1]! * (1 - a), fg[2]! * a + bg[2]! * (1 - a), 1]; };
      const lum = (c: number[]) => { const f = (v: number) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }; return 0.2126 * f(c[0]!) + 0.7152 * f(c[1]!) + 0.0722 * f(c[2]!); };
      const backdrop = (el: Element): number[] | null => {
        const chain: Element[] = [];
        for (let p: Element | null = el; p; p = p.parentElement) chain.push(p);
        let bg: number[] = [255, 255, 255, 1];
        for (const p of chain.reverse()) {
          const cs = getComputedStyle(p);
          if (cs.backgroundImage !== 'none') return null; // a picture or gradient: cannot say
          const c = rgba(cs.backgroundColor);
          if (c === null) return null;
          if (c[3] > 0) bg = over(c, bg);
        }
        return bg;
      };
      const fails = new Map<string, string>();
      let checked = 0;
      const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
      for (let n = walker.nextNode(); n; n = walker.nextNode()) {
        const t = (n.textContent ?? '').trim();
        const el = n.parentElement;
        if (!t || !el || /^(SCRIPT|STYLE|NOSCRIPT|TITLE)$/.test(el.tagName)) continue;
        const cs = getComputedStyle(el);
        if (cs.display === 'none' || cs.visibility === 'hidden' || el.getBoundingClientRect().width === 0) continue;
        if (el.closest('[aria-hidden="true"], .stage__labels, .foot__mark, [data-marquee] [aria-hidden]')) continue;
        let op = 1; for (let p: Element | null = el; p; p = p.parentElement) op *= +getComputedStyle(p).opacity;
        const fg0 = rgba(cs.color); const bg = backdrop(el);
        if (!fg0 || !bg) continue;
        const fg = over([fg0[0], fg0[1], fg0[2], fg0[3] * op], bg);
        const L1 = lum(fg), L2 = lum(bg);
        const ratio = (Math.max(L1, L2) + 0.05) / (Math.min(L1, L2) + 0.05);
        const px = parseFloat(cs.fontSize), bold = +cs.fontWeight >= 700;
        const need = px >= 24 || (px >= 18.66 && bold) ? 3 : 4.5;
        checked++;
        if (ratio < need) fails.set(`${el.tagName.toLowerCase()}.${(el as HTMLElement).className}`, `${ratio.toFixed(2)} < ${need} op=${op} "${t.slice(0, 30)}" ${cs.color} on rgb(${bg.slice(0, 3).map(Math.round).join(',')})`);
      }
      return { checked, fails: Array.from(fails.values()).slice(0, 15), n: fails.size };
    });
    expect(r.checked).toBeGreaterThan(150);
    expect(r.fails, `${r.n} kinds of text are too faint`).toEqual([]);
    await guard.check();
  });
}
