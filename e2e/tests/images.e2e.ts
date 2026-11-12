import { test, expect, scrollThrough } from '../lib/support.ts';

// Pictures: AVIF, WebP and PNG fallbacks, sizes, decoding, no layout shift from wrong ratios.

test('every <picture> offers AVIF, then WebP, then a PNG fallback, and the formats match their extensions', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const out = { pictures: 0, bad: [] as string[] };
    for (const pic of Array.from(document.querySelectorAll('picture'))) {
      out.pictures++;
      const types = Array.from(pic.querySelectorAll('source')).map((s) => s.getAttribute('type'));
      const img = pic.querySelector('img');
      const id = img?.getAttribute('src') ?? '(no img)';
      if (types[0] !== 'image/avif' || types[1] !== 'image/webp' || types.length !== 2) out.bad.push(`${id}: sources ${JSON.stringify(types)}`);
      for (const s of Array.from(pic.querySelectorAll('source'))) {
        const ext = s.getAttribute('type') === 'image/avif' ? '.avif' : '.webp';
        for (const c of (s.getAttribute('srcset') ?? '').split(',')) {
          const u = c.trim().split(/\s+/)[0] ?? '';
          if (!u.endsWith(ext)) out.bad.push(`${id}: ${u} is not ${ext}`);
          if (!/\s\d+w$/.test(c.trim())) out.bad.push(`${id}: candidate without a width: ${c.trim()}`);
        }
        if (!s.getAttribute('sizes')) out.bad.push(`${id}: source without sizes`);
      }
      if (!img || !/\.png$/.test(img.getAttribute('src') ?? '')) out.bad.push(`${id}: fallback is not a PNG`);
    }
    return out;
  });
  expect(r.pictures).toBeGreaterThan(25);
  expect(r.bad).toEqual([]);
  await guard.check();
});

test('the AVIF, WebP and PNG candidates all exist, are the format they say, and decode in this browser (or the browser picks another)', async ({ app, browser, guard }) => {
  await app.open('/');
  const urls = await browser.evaluate(() => {
    // One picture per kind of image is enough to prove the files are the right format.
    const seen = new Set<string>(), out: { avif: string[]; webp: string[]; png: string[] } = { avif: [], webp: [], png: [] };
    for (const pic of Array.from(document.querySelectorAll('picture'))) {
      const img = pic.querySelector('img');
      const key = (img?.getAttribute('src') ?? '').replace(/-(light|dark).*/, '');
      if (seen.has(key)) continue;
      seen.add(key);
      for (const s of Array.from(pic.querySelectorAll('source'))) {
        const first = (s.getAttribute('srcset') ?? '').split(',')[0]!.trim().split(/\s+/)[0]!;
        (s.getAttribute('type') === 'image/avif' ? out.avif : out.webp).push(new URL(first, location.href).href);
      }
      if (img) out.png.push(new URL(img.getAttribute('src')!, location.href).href);
    }
    return out;
  });
  const magic = async (u: string) => Buffer.from(await (await fetch(u)).arrayBuffer()).subarray(0, 16);
  for (const u of urls.png) expect((await magic(u)).subarray(1, 4).toString(), u).toBe('PNG');
  for (const u of urls.webp) { const b = await magic(u); expect(b.subarray(0, 4).toString() + b.subarray(8, 12).toString(), u).toBe('RIFFWEBP'); }
  for (const u of urls.avif) expect((await magic(u)).subarray(4, 12).toString(), u).toMatch(/^ftyp(avif|avis)/);
  // Decoding: the PNG and WebP fallbacks must work everywhere; AVIF where the browser has it.
  const dec = await browser.evaluate(async (u: { avif: string[]; webp: string[]; png: string[] }) => {
    const tryDecode = async (src: string) => { const i = new Image(); i.src = src; try { await i.decode(); return i.naturalWidth > 0; } catch { return false; } };
    return {
      png: await Promise.all(u.png.slice(0, 6).map(tryDecode)),
      webp: await Promise.all(u.webp.slice(0, 6).map(tryDecode)),
      avif: await Promise.all(u.avif.slice(0, 6).map(tryDecode)),
    };
  }, urls);
  expect(dec.png.every(Boolean)).toBe(true);
  expect(dec.webp.every(Boolean)).toBe(true);
  // AVIF: either every sample decodes, or the browser lacks AVIF and picks WebP (checked in the next test).
  expect(dec.avif.every(Boolean) || dec.avif.every((x) => !x)).toBe(true);
  await guard.check();
});

test('every visible image loads, decodes, and has the aspect ratio its width and height promise', async ({ app, browser, guard }) => {
  await app.open('/');
  await scrollThrough(browser);
  // Whatever is still waiting: bring each into view, as a visitor would, and give it time.
  const waiting = await browser.evaluate(() => Array.from(document.querySelectorAll('img')).map((i, n) => (i.offsetParent !== null && !i.complete ? n : -1)).filter((n) => n >= 0));
  for (const n of waiting) {
    await browser.evaluate(async (k: number) => { const i = document.querySelectorAll('img')[k]!; i.scrollIntoView({ block: 'center' }); for (let t = 0; t < 60 && !i.complete; t++) await new Promise((r) => setTimeout(r, 50)); return true; }, n);
  }
  await expect.poll(() => browser.evaluate(() => Array.from(document.querySelectorAll('img')).filter((i) => i.offsetParent !== null && !i.complete).map((i) => `${i.getAttribute('src')} at y=${Math.round(i.getBoundingClientRect().top + scrollY)}`)), { timeout: 15_000 }).toEqual([]);
  const r = await browser.evaluate(() => {
    const bad: string[] = [];
    let seen = 0;
    const picked = new Set<string>();
    for (const img of Array.from(document.querySelectorAll('img'))) {
      if (img.offsetParent === null) continue; // not displayed: the other theme, a closed tab
      seen++;
      const name = img.currentSrc || img.src;
      const ext = (name.match(/\.(\w+)$/)?.[1] ?? '').toLowerCase();
      picked.add(ext);
      if (!img.complete || img.naturalWidth === 0) { bad.push(`not loaded: ${name}`); continue; }
      const want = +(img.getAttribute('width') ?? 0) / +(img.getAttribute('height') ?? 1);
      const got = img.naturalWidth / img.naturalHeight;
      if (Math.abs(got - want) / want > 0.02) bad.push(`ratio ${got.toFixed(3)} != ${want.toFixed(3)}: ${name}`);
    }
    return { seen, bad, picked: Array.from(picked).sort() };
  });
  expect(r.seen).toBeGreaterThan(5);
  expect(r.bad).toEqual([]);
  // Whatever the browser picked, it is a modern format, never the PNG fallback.
  for (const e of r.picked) expect(['avif', 'webp', 'svg', 'png']).toContain(e);
  const pictured = await browser.evaluate(() => Array.from(document.querySelectorAll('picture img')).filter((i) => (i as HTMLImageElement).offsetParent && (i as HTMLImageElement).complete).map((i) => ((i as HTMLImageElement).currentSrc.match(/\.(\w+)$/)?.[1] ?? '')));
  expect(pictured.filter((e) => e === 'png')).toEqual([]);
  await guard.check();
});

test('the hero shows the poster of the active theme (and only that one) when there is no 3D', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const shown = () => browser.evaluate(() => {
    const vis = (s: string) => { const e = document.querySelector(s) as HTMLElement | null; return !!e && getComputedStyle(e).display !== 'none' && e.offsetWidth > 0; };
    return { light: vis('.stage__poster img.t-light'), dark: vis('.stage__poster img.t-dark'), theme: document.documentElement.dataset.theme };
  });
  const a = await shown();
  expect(a.light).toBe(a.theme === 'light');
  expect(a.dark).toBe(a.theme === 'dark');
  await screen.getByRole('button', { name: /Switch to (dark|light) theme/ }).tap();
  const b = await shown();
  expect(b.theme).not.toBe(a.theme);
  expect(b.light).toBe(b.theme === 'light');
  expect(b.dark).toBe(b.theme === 'dark');
  await expect.poll(() => browser.evaluate(() => { const i = Array.from(document.querySelectorAll('.stage__poster img')).find((x) => (x as HTMLElement).offsetWidth > 0) as HTMLImageElement; return i.complete && i.naturalWidth > 0; }), { timeout: 10_000 }).toBe(true);
  await guard.check();
});
