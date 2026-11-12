import { test, expect, currentProfile, statuses, statusOf } from '../lib/support.ts';

// What the page is made of: head, landmarks, headings, anchors, links, text.

test('the head is complete: title, description, icons, canonical, social card, structured data', async ({ app, browser, guard }) => {
  await app.open('/');
  const head = await browser.evaluate(() => {
    const q = (s: string) => document.querySelector(s);
    const attr = (s: string, a: string) => q(s)?.getAttribute(a) ?? null;
    let ld: unknown = null;
    try { ld = JSON.parse(q('script[type="application/ld+json"]')?.textContent ?? 'null'); } catch { ld = 'invalid'; }
    return {
      lang: document.documentElement.lang,
      title: document.title,
      description: attr('meta[name="description"]', 'content'),
      viewport: attr('meta[name="viewport"]', 'content'),
      canonical: attr('link[rel="canonical"]', 'href'),
      ogImage: attr('meta[property="og:image"]', 'content'),
      ogTitle: attr('meta[property="og:title"]', 'content'),
      twitterCard: attr('meta[name="twitter:card"]', 'content'),
      colorScheme: attr('meta[name="color-scheme"]', 'content'),
      icons: Array.from(document.querySelectorAll('link[rel~="icon"], link[rel="apple-touch-icon"]')).map((l) => l.getAttribute('href')),
      ldOk: ld !== 'invalid' && ld !== null,
      h1: document.querySelectorAll('h1').length,
    };
  });
  expect(head.lang).toBe('en');
  expect(head.title).toMatch(/oneMK/);
  expect(head.title.length).toBeLessThan(70);
  expect((head.description ?? '').length).toBeGreaterThan(60);
  expect((head.description ?? '').length).toBeLessThan(320);
  expect(head.viewport).toContain('width=device-width');
  expect(head.canonical).toBe('https://samer2373.github.io/one-mk/');
  expect(head.ogImage).toBe('https://samer2373.github.io/one-mk/assets/img/og.png');
  expect(head.ogTitle).toMatch(/oneMK/);
  expect(head.twitterCard).toBe('summary_large_image');
  expect(head.colorScheme).toBe('light dark');
  expect(head.icons.length).toBeGreaterThanOrEqual(3);
  expect(head.ldOk).toBe(true);
  expect(head.h1).toBe(1);
  await guard.check();
});

test('the social card, icons and manifest exist and are what they claim to be', async ({ app }) => {
  const base = app.baseUrl;
  const og = await fetch(`${base}/assets/img/og.png`);
  expect(og.status).toBe(200);
  const buf = Buffer.from(await og.arrayBuffer());
  expect(buf.subarray(1, 4).toString()).toBe('PNG');
  expect(buf.readUInt32BE(16)).toBe(1200);
  expect(buf.readUInt32BE(20)).toBe(630);
  for (const f of ['favicon.svg', 'favicon.ico', 'apple-touch-icon.png', 'robots.txt', 'sitemap.xml', 'site.webmanifest']) {
    const r = await fetch(`${base}/${f}`);
    expect(r.status, f).toBe(200);
  }
  const manifest = await (await fetch(`${base}/site.webmanifest`)).json() as { name: string; icons: { src: string }[] };
  expect(manifest.name).toMatch(/oneMK/);
  for (const i of manifest.icons) expect((await fetch(`${base}/${i.src.replace(/^\//, '')}`)).status, i.src).toBe(200);
  const sitemap = await (await fetch(`${base}/sitemap.xml`)).text();
  expect(sitemap).toContain('https://samer2373.github.io/one-mk/');
});

test('landmarks and headings: one h1, a banner, a main, a footer, no skipped heading level', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const heads = Array.from(document.querySelectorAll('h1,h2,h3,h4,h5,h6')).map((h) => ({ level: +h.tagName[1], text: (h.textContent ?? '').trim(), hidden: !(h as HTMLElement).offsetParent && getComputedStyle(h).position !== 'fixed' }));
    return {
      banner: document.querySelectorAll('header.bar').length,
      main: document.querySelectorAll('main#main').length,
      footer: document.querySelectorAll('footer').length,
      navs: Array.from(document.querySelectorAll('nav')).map((n) => n.getAttribute('aria-label')),
      heads,
      emptyHeads: heads.filter((h) => !h.text).length,
    };
  });
  expect(r.banner).toBe(1);
  expect(r.main).toBe(1);
  expect(r.footer).toBe(1);
  expect(r.navs).toEqual(['Sections', 'Project', 'Trust']);
  expect(r.emptyHeads).toBe(0);
  let prev = 0;
  for (const h of r.heads) {
    expect(h.level - prev, `heading "${h.text}" jumps from h${prev} to h${h.level}`).toBeLessThanOrEqual(1);
    prev = h.level;
  }
  expect(r.heads[0]).toMatchObject({ level: 1, text: 'One mouse. One keyboard. Every computer.' });
  await guard.check();
});

test('every section has a heading and a label that points at it; ids are unique', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const ids = Array.from(document.querySelectorAll('[id]')).map((e) => e.id);
    const dupes = ids.filter((id, i) => ids.indexOf(id) !== i);
    const sections = Array.from(document.querySelectorAll('main > section')).map((s) => {
      const lb = s.getAttribute('aria-labelledby') ?? s.getAttribute('aria-label') ?? '';
      const target = s.getAttribute('aria-labelledby') ? document.getElementById(lb) : null;
      return { id: s.id, cls: s.className, labelled: !!lb, labelOk: s.getAttribute('aria-labelledby') ? !!target && (target.textContent ?? '').trim().length > 0 : true };
    });
    const dangling = Array.from(document.querySelectorAll('[aria-labelledby],[aria-controls],[aria-describedby]')).flatMap((e) =>
      ['aria-labelledby', 'aria-controls', 'aria-describedby'].flatMap((a) => (e.getAttribute(a) ?? '').split(/\s+/).filter(Boolean).filter((id) => !document.getElementById(id)).map((id) => `${a}=${id}`)));
    return { dupes, sections, dangling };
  });
  expect(r.dupes).toEqual([]);
  expect(r.dangling).toEqual([]);
  expect(r.sections.map((s) => s.cls)).toEqual(['hero', 'facts', 'how', 'features', 'remap', 'gallery', 'security', 'platforms', 'download', 'faq', 'cta', 'maker']);
  for (const s of r.sections) { expect(s.labelled, s.cls).toBe(true); expect(s.labelOk, s.cls).toBe(true); }
  await guard.check();
});

test('every in-page link lands on an element that exists', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const hashes = Array.from(document.querySelectorAll('a[href^="#"]')).map((a) => (a as HTMLAnchorElement).getAttribute('href') as string);
    const uniq = Array.from(new Set(hashes));
    return { uniq, missing: uniq.filter((h) => h !== '#' && !document.getElementById(h.slice(1))), count: hashes.length };
  });
  expect(r.missing).toEqual([]);
  expect(r.uniq).toEqual(expect.arrayContaining(['#top', '#how', '#features', '#security', '#download', '#faq', '#main']));
  expect(r.uniq).not.toContain('#');
  await guard.check();
});

test('every link has a name, a destination, and external links are safe and on the expected hosts', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const out: { bad: string[]; hosts: string[]; total: number; external: number } = { bad: [], hosts: [], total: 0, external: 0 };
    const hosts = new Set<string>();
    for (const a of Array.from(document.querySelectorAll('a'))) {
      out.total++;
      const href = a.getAttribute('href') ?? '';
      const name = (a.getAttribute('aria-label') ?? a.textContent ?? '').replace(/\s+/g, ' ').trim();
      const where = `${name || '(no name)'} -> ${href}`;
      if (!href) out.bad.push(`no href: ${where}`);
      if (/^(javascript:|#$)/.test(href)) out.bad.push(`dead href: ${where}`);
      if (!name) out.bad.push(`no accessible name: ${href}`);
      if (/^https?:/.test(href)) {
        out.external++;
        hosts.add(new URL(href).host);
        if (a.target === '_blank' && !/noopener/.test(a.rel)) out.bad.push(`target=_blank without noopener: ${where}`);
        if (!/^https:/.test(href)) out.bad.push(`not https: ${where}`);
      }
    }
    out.hosts = Array.from(hosts).sort();
    return out;
  });
  expect(r.bad).toEqual([]);
  expect(r.hosts).toEqual(['bigfootjo.com', 'creativecommons.org', 'github.com', 'raw.githubusercontent.com', 'sketchfab.com'].filter((h) => r.hosts.includes(h)));
  expect(r.hosts).toEqual(expect.arrayContaining(['github.com', 'bigfootjo.com']));
  // Links that leave for GitHub all stay inside the project, except the licence credits.
  const gh = await browser.evaluate(() => Array.from(document.querySelectorAll('a[href^="https://github.com/"]')).map((a) => (a as HTMLAnchorElement).pathname));
  for (const p of gh) expect(p, p).toMatch(/^\/samer2373\/one-mk(\/|$)/);
  await guard.check();
});

test('every local file the HTML and CSS point at exists', async ({ app, browser, guard }) => {
  await app.open('/');
  const base = app.baseUrl;
  const refs = await browser.evaluate(() => {
    const out = new Set<string>();
    const add = (v: string | null) => { if (v && !/^(#|data:|mailto:|javascript:)/.test(v)) out.add(new URL(v, location.href).href); };
    document.querySelectorAll('[src],[href],[poster]').forEach((e) => { add(e.getAttribute('src')); if (e.tagName !== 'A') add(e.getAttribute('href')); add(e.getAttribute('poster')); });
    document.querySelectorAll('[srcset]').forEach((e) => (e.getAttribute('srcset') ?? '').split(',').forEach((c) => add(c.trim().split(/\s+/)[0] ?? null)));
    document.querySelectorAll('a[href]').forEach((a) => { const h = a.getAttribute('href') ?? ''; if (!/^(https?:|#|mailto:)/.test(h)) add(h); });
    return Array.from(out);
  });
  const local = refs.filter((u) => u.startsWith(base));
  expect(local.length).toBeGreaterThan(40);
  const got = await statuses(local);
  expect(Object.entries(got).filter(([, s]) => s !== 200).map(([u, s]) => `${s} ${u}`)).toEqual([]);
  // CSS: every url() in the stylesheet.
  const css = await (await fetch(`${base}/css/site.css`)).text();
  for (const m of css.matchAll(/url\(\s*["']?([^)"']+)["']?\s*\)/g)) {
    const u = m[1]!;
    if (u.startsWith('data:') || u.startsWith('#')) continue;
    expect(await statusOf(new URL(u, `${base}/css/`).href), `css url(${u})`).toBe(200);
  }
  await guard.check();
});

test('the words on the page are sound: no leftovers, no broken characters, no em dashes, the version is consistent', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const text = document.body.innerText;
    const html = document.documentElement.outerHTML;
    const versions = Array.from(document.querySelectorAll('body [data-version]')).map((e) => (e.textContent ?? '').trim());
    const versionAttr = document.documentElement.dataset.version ?? '';
    return {
      leftovers: (text.match(/undefined|\bnull\b|NaN|\[object|\{\{|\}\}|lorem ipsum|TODO|FIXME|XXX/gi) ?? []),
      mojibake: (html.match(/�|Ã.|â€|Â /g) ?? []),
      emDash: (text.match(/—/g) ?? []).length,
      doubleSpace: (text.match(/[A-Za-z.,]  +[A-Za-z]/g) ?? []),
      versions,
      versionAttr,
      brand: (text.match(/one-?mk|onemk/gi) ?? []),
    };
  });
  expect(r.leftovers).toEqual([]);
  expect(r.mojibake).toEqual([]);
  expect(r.emDash).toBe(0);
  expect(r.doubleSpace).toEqual([]);
  expect(r.versions.length).toBeGreaterThanOrEqual(2);
  for (const v of r.versions) expect(v).toBe(r.versionAttr);
  // The product is always written oneMK in prose (the program and command are one-mk).
  const wrong = r.brand.filter((b) => b !== 'oneMK' && b !== 'one-mk');
  expect(wrong).toEqual([]);
  await guard.check();
});

test('images: every one has alt text (empty only when decorative), a size, and lazy loading below the fold', async ({ app, browser, guard }) => {
  await app.open('/');
  const r = await browser.evaluate(() => {
    const imgs = Array.from(document.querySelectorAll('img'));
    const noAlt = imgs.filter((i) => i.getAttribute('alt') === null).map((i) => i.getAttribute('src'));
    const noSize = imgs.filter((i) => !i.getAttribute('width') || !i.getAttribute('height')).map((i) => i.getAttribute('src'));
    const informative = imgs.filter((i) => (i.getAttribute('alt') ?? '').length > 0);
    const shortAlt = informative.filter((i) => (i.getAttribute('alt') ?? '').length < 12).map((i) => i.alt);
    const eager = imgs.filter((i) => i.loading !== 'lazy').map((i) => i.getAttribute('src'));
    return { total: imgs.length, noAlt, noSize, shortAlt, eager };
  });
  expect(r.total).toBeGreaterThan(30);
  expect(r.noAlt).toEqual([]);
  expect(r.noSize).toEqual([]);
  expect(r.shortAlt).toEqual([]);
  // Only the little brand tile in the bar is eager.
  expect(r.eager.every((s) => /bigfoot-mark/.test(s ?? ''))).toBe(true);
  await guard.check();
  const p = await currentProfile(browser);
  expect(p.name).toBeTruthy();
});
