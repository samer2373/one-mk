import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { test, expect, currentProfile, SITE_DIR } from '../lib/support.ts';

// The FAQ, the "install steps" folds in the download cards, and the copy buttons.

const QUESTIONS = [
  'Is oneMK free?',
  'Do my computers need the internet?',
  'What gets shared, exactly?',
  'What if the pointer gets stuck on another computer?',
  'Can I use it with a Mac and a Linux PC together?',
  'Does it work at the Windows sign-in screen?',
  'How do updates work?',
  'Can I drive it from a terminal?',
];

test('the FAQ lists every question, all closed, each with a non-empty answer', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => Array.from(document.querySelectorAll('.faq details')).map((d) => ({
    q: (d.querySelector('summary')?.textContent ?? '').trim(), open: (d as HTMLDetailsElement).open, a: (d.querySelector('p')?.textContent ?? '').trim().length,
  })));
  expect(r.map((x) => x.q)).toEqual(QUESTIONS);
  expect(r.every((x) => !x.open)).toBe(true);
  expect(r.every((x) => x.a > 40)).toBe(true);
  await guard.check();
});

test('a question opens and closes on a click, and several can be open together', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const faq = browser.locator('.faq__list');
  const isOpen = (i: number) => browser.evaluate((k: number) => (document.querySelectorAll('.faq details')[k] as HTMLDetailsElement).open, i);
  const answerVisible = (i: number) => browser.evaluate((k: number) => { const p = document.querySelectorAll('.faq details')[k]!.querySelector('p') as HTMLElement; return p.checkVisibility() && p.getBoundingClientRect().height > 0; }, i);
  await faq.scrollIntoView();
  const q0 = screen.getByText(QUESTIONS[0]!, { exact: true });
  const q2 = screen.getByText(QUESTIONS[2]!, { exact: true });
  expect(await answerVisible(0)).toBe(false);
  await q0.tap();
  expect(await isOpen(0)).toBe(true);
  expect(await answerVisible(0)).toBe(true);
  await q2.tap();
  expect(await isOpen(0)).toBe(true);
  expect(await isOpen(2)).toBe(true);
  await q0.tap();
  expect(await isOpen(0)).toBe(false);
  expect(await answerVisible(0)).toBe(false);
  expect(await isOpen(2)).toBe(true);
  await guard.check();
});

test('a question works from the keyboard: Tab to it, Enter opens, Space closes', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const summary = browser.locator('.faq details >> nth=3 >> summary');
  await summary.scrollIntoView();
  await summary.focus();
  await expect(summary).toBeFocused();
  const isOpen = () => browser.evaluate<boolean>("document.querySelectorAll('.faq details')[3].open");
  await browser.keyboard.press('Enter');
  expect(await isOpen()).toBe(true);
  await browser.keyboard.press('Space');
  expect(await isOpen()).toBe(false);
  // The focus ring is drawn on the summary.
  const ring = await browser.evaluate(() => { const s = getComputedStyle(document.activeElement as Element); return { outline: s.outlineStyle, w: parseFloat(s.outlineWidth), shadow: s.boxShadow }; });
  expect(ring.outline !== 'none' && ring.w > 0 || ring.shadow !== 'none').toBe(true);
  await guard.check();
});

test('answers that contain links, keys and commands keep them readable and inside the column', async ({ app, browser, guard }) => {
  await app.open('/');
  await browser.evaluate("document.querySelectorAll('.faq details').forEach((d) => { d.open = true; })");
  const r = await browser.evaluate(() => {
    const col = (document.querySelector('.faq__list') as HTMLElement).getBoundingClientRect();
    const out: string[] = [];
    document.querySelectorAll('.faq details p').forEach((p) => {
      const pr = p.getBoundingClientRect();
      if (pr.right > col.right + 1 || pr.left < col.left - 1) out.push(`p overflows the column: ${(p.textContent ?? '').slice(0, 40)}`);
      p.querySelectorAll('code, kbd, a').forEach((e) => { const r = e.getBoundingClientRect(); if (r.right > document.documentElement.clientWidth + 1) out.push(`${e.tagName} ${e.textContent} off screen`); });
    });
    return { out, links: Array.from(document.querySelectorAll('.faq a')).map((a) => (a as HTMLAnchorElement).href) };
  });
  expect(r.out).toEqual([]);
  expect(r.links).toEqual(['https://github.com/samer2373/one-mk/blob/main/docs/COMMANDS.md']);
  await guard.check();
});

test('the install steps in each download card fold open and keep their commands inside the card', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(async () => {
    const out: { panel: string; opened: boolean; overflow: number; pre: boolean }[] = [];
    for (const card of Array.from(document.querySelectorAll('[data-panel]'))) {
      const d = card.querySelector('details.dl-more') as HTMLDetailsElement;
      d.open = true;
      await new Promise((r) => setTimeout(r, 50));
      const cr = (card as HTMLElement).getBoundingClientRect();
      let worst = 0;
      // A command may scroll inside its own box (pre), so measure the boxes, not the text that scrolls in them.
      card.querySelectorAll('li, p, .cmd, pre, code').forEach((e) => { if (e.tagName === 'CODE' && e.closest('pre')) return; const r = e.getBoundingClientRect(); worst = Math.max(worst, r.right - cr.right); });
      out.push({ panel: (card as HTMLElement).dataset.panel!, opened: d.open, overflow: Math.round(worst), pre: !!card.querySelector('pre') });
    }
    return out;
  });
  expect(r.map((x) => x.panel).sort()).toEqual(['linux', 'mac', 'windows']);
  for (const x of r) expect(x.overflow, `${x.panel} card content sticks out by ${x.overflow}px`).toBeLessThanOrEqual(1);
  await guard.check();
});

const COPY = [
  { which: 'mac', cmd: 'curl -fsSL https://raw.githubusercontent.com/samer2373/one-mk/main/install.sh | sh', label: 'Copy the install command' },
  { which: 'linux', cmd: 'curl -fsSL https://raw.githubusercontent.com/samer2373/one-mk/main/install.sh | sh', label: 'Copy the install command' },
  { which: 'sums', cmd: 'sha256sum -c --ignore-missing SHA256SUMS', label: 'Copy the checksum command' },
];

test('each copy button copies exactly the command beside it and says so for a moment', async ({ app, browser, guard }) => {
  const p0 = await (async () => { await app.open('/'); return currentProfile(browser); })();
  // Record what the page hands to the clipboard, and still call the real one.
  await browser.addInitScript(`(function(){
    window.__copied = [];
    var c = navigator.clipboard;
    if (!c) return;
    var real = c.writeText ? c.writeText.bind(c) : null;
    try { Object.defineProperty(c, 'writeText', { value: function(t){ window.__copied.push(t); return real ? real(t).catch(function(){}) : Promise.resolve(); }, configurable: true }); } catch (e) {}
  })();`);
  await app.open('/');
  expect(existsSync(join(SITE_DIR, '..', 'install.sh'))).toBe(true);
  const n = await browser.evaluate<number>("document.querySelectorAll('[data-copy]').length");
  expect(n).toBe(3);
  for (const [i, c] of COPY.entries()) {
    const sel = `[data-copy] >> nth=${i}`;
    await browser.evaluate((k: number) => { const b = document.querySelectorAll('[data-copy]')[k]!; const d = b.closest('details'); if (d) d.open = true; return true; }, i);
    const btn = browser.locator(sel);
    await btn.scrollIntoView();
    await expect(btn).toHaveAttribute('aria-label', c.label);
    await btn.tap();
    await expect(btn).toHaveAttribute('aria-label', 'Copied');
    const st = await browser.evaluate((k: number) => {
      const b = document.querySelectorAll('[data-copy]')[k] as HTMLElement;
      return { done: b.classList.contains('is-done'), icon: b.querySelector('use')?.getAttribute('href'), code: (b.parentElement!.querySelector('code')?.textContent ?? ''), copied: (window as unknown as { __copied: string[] }).__copied.at(-1) ?? null };
    }, i);
    expect(st.done).toBe(true);
    expect(st.icon).toBe('#i-check');
    expect(st.code).toBe(c.cmd);
    if (p0.clipboard !== 'none' || st.copied !== null) expect(st.copied).toBe(c.cmd);
    // It goes back to a copy button after about 1.6 s.
    await expect(btn).toHaveAttribute('aria-label', c.label, { timeout: 5000 });
    const after = await browser.evaluate((k: number) => { const b = document.querySelectorAll('[data-copy]')[k] as HTMLElement; return { done: b.classList.contains('is-done'), icon: b.querySelector('use')?.getAttribute('href') }; }, i);
    expect(after).toEqual({ done: false, icon: '#i-copy' });
  }
  await guard.check();
});

test('the real clipboard holds the copied command (where the browser lets a page read it back)', async ({ app, browser, guard }) => {
  await app.open('/');
  const p = await currentProfile(browser);
  test.skip(p.clipboard === 'none', 'this browser cannot grant a page clipboard-read in automation');
  const idx = 2;
  await browser.evaluate((k: number) => { const b = document.querySelectorAll('[data-copy]')[k]!; (b.closest('details') as HTMLDetailsElement | null)?.setAttribute('open', ''); return true; }, idx);
  const btn = browser.locator(`[data-copy] >> nth=${idx}`);
  await btn.scrollIntoView();
  await btn.tap();
  await expect(btn).toHaveAttribute('aria-label', 'Copied');
  await expect.poll(() => browser.evaluate<string>('navigator.clipboard.readText().catch(function(){ return "unreadable"; })'), { timeout: 5000 }).toBe(COPY[idx]!.cmd);
  await guard.check();
});

test('with no async clipboard (an old browser, or plain http) the copy button falls back and still reports success', async ({ app, browser, guard }) => {
  await browser.addInitScript(`try { Object.defineProperty(navigator, 'clipboard', { value: undefined, configurable: true }); } catch (e) {}
    window.__exec = []; var e = document.execCommand ? document.execCommand.bind(document) : null;
    document.execCommand = function(c){ window.__exec.push(c + ':' + (document.activeElement && document.activeElement.value)); return e ? e.apply(document, arguments) : true; };`);
  await app.open('/');
  await browser.evaluate("document.querySelectorAll('[data-copy]')[2].closest('details') && (document.querySelectorAll('[data-copy]')[2].closest('details').open = true)");
  const btn = browser.locator('[data-copy] >> nth=2');
  await btn.scrollIntoView();
  await btn.tap();
  await expect(btn).toHaveAttribute('aria-label', 'Copied');
  const exec = await browser.evaluate<string[]>('window.__exec');
  expect(exec).toEqual(['copy:' + COPY[2]!.cmd]);
  // The scratch text box it made is gone again.
  expect(await browser.evaluate<number>("document.querySelectorAll('textarea').length")).toBe(0);
  await guard.check();
});
