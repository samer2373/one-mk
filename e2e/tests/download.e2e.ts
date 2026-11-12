import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test, test3d, expect, pretend, detected, SITE_DIR } from '../lib/support.ts';
import { FILES, RELEASE, UA, type EnvSpec } from '../lib/profiles.ts';

// The download chooser (js/detect.js and js/site.js) as a visitor on each kind of computer sees it.

interface Case {
  name: string;
  env: EnvSpec;
  panel: 'mac' | 'windows' | 'linux' | null;
  file: string | null;
  label: string;
  sure: boolean;
  /** The file marked as the recommended one inside the detected card ([] when nothing is marked). */
  rec: string[];
  /** Words the card's explanation must contain, or null for no explanation. */
  why?: RegExp | null;
  /** Words the note under the hero button must contain. */
  note?: RegExp;
  mobile?: boolean;
  gl?: boolean;
}

const MAC_CH = { platform: 'macOS', mobile: false };
const WIN_CH = { platform: 'Windows', mobile: false };
const LIN_CH = { platform: 'Linux', mobile: false };

const CASES: Case[] = [
  { name: 'Mac Chrome, Apple silicon (client hints)', env: { ua: UA.macChrome, platform: 'MacIntel', touch: 0, uaData: MAC_CH, hints: { architecture: 'arm', bitness: '64' } },
    panel: 'mac', file: FILES.macArm, label: 'Download for macOS (Apple silicon)', sure: true, rec: [FILES.macArm], why: /reports an Apple silicon Mac/, note: /For Apple silicon/ },
  { name: 'Mac Chrome, Intel (client hints)', env: { ua: UA.macChrome, platform: 'MacIntel', touch: 0, uaData: MAC_CH, hints: { architecture: 'x86', bitness: '64' } },
    panel: 'mac', file: FILES.macIntel, label: 'Download for macOS (Intel)', sure: true, rec: [FILES.macIntel], why: /reports an Intel Mac/, note: /For Intel Macs/ },
  { name: 'Mac Safari, Apple GPU (graphics chip says Apple silicon)', env: { ua: UA.macSafari, platform: 'MacIntel', touch: 0, gl: 'Apple GPU' },
    panel: 'mac', file: FILES.macArm, label: 'Download for macOS (Apple silicon)', sure: false, rec: [FILES.macArm], why: /Looks like Apple silicon/, note: /Looks like Apple silicon/, gl: true },
  { name: 'Mac Safari, Intel graphics', env: { ua: UA.macSafari, platform: 'MacIntel', touch: 0, gl: 'Intel(R) Iris(TM) Plus Graphics 655' },
    panel: 'mac', file: FILES.macIntel, label: 'Download for macOS (Intel)', sure: false, rec: [FILES.macIntel], why: /Looks like an Intel Mac/, note: /Looks like an Intel Mac/, gl: true },
  { name: 'Mac Firefox, nothing known about the chip', env: { ua: UA.macFirefox, platform: 'MacIntel', touch: 0 },
    panel: 'mac', file: FILES.macArm, label: 'Download for macOS (Apple silicon)', sure: false, rec: [], why: /doesn’t say whether this Mac has Apple silicon or Intel/, note: /Apple silicon \(M1 and later\)/ },
  { name: 'Windows 10 Chrome', env: { ua: UA.winChrome, platform: 'Win32', touch: 0, uaData: WIN_CH, hints: { architecture: 'x86', bitness: '64', platformVersion: '10.0.0' } },
    panel: 'windows', file: FILES.winSetup, label: 'Download for Windows', sure: true, rec: [FILES.winSetup], why: null, note: /Windows 10, 64-bit installer/ },
  { name: 'Windows 11 Edge', env: { ua: UA.winEdge, platform: 'Win32', touch: 0, uaData: WIN_CH, hints: { architecture: 'x86', bitness: '64', platformVersion: '15.0.0' } },
    panel: 'windows', file: FILES.winSetup, label: 'Download for Windows', sure: true, rec: [FILES.winSetup], why: null, note: /Windows 11, 64-bit installer/ },
  { name: 'Windows Firefox', env: { ua: UA.winFirefox, platform: 'Win32', touch: 0 },
    panel: 'windows', file: FILES.winSetup, label: 'Download for Windows', sure: false, rec: [FILES.winSetup], why: null, note: /Windows 10 and 11, 64-bit installer/ },
  { name: 'Windows on ARM (client hints)', env: { ua: UA.winChrome, platform: 'Win32', touch: 0, uaData: WIN_CH, hints: { architecture: 'arm', bitness: '64', platformVersion: '15.0.0' } },
    panel: 'windows', file: null, label: 'See the Windows downloads', sure: true, rec: [], why: /no ARM build yet/, note: /No build for Windows on ARM/ },
  { name: 'Ubuntu Firefox', env: { ua: UA.ubuntuFirefox, platform: 'Linux x86_64', touch: 0 },
    panel: 'linux', file: FILES.deb, label: 'Download for Linux (.deb)', sure: true, rec: [FILES.deb], why: null, note: /Debian, Ubuntu and relatives/ },
  { name: 'Fedora Firefox (the AppImage)', env: { ua: UA.fedoraFirefox, platform: 'Linux x86_64', touch: 0 },
    panel: 'linux', file: FILES.appImage, label: 'Download for Linux (AppImage)', sure: true, rec: [FILES.appImage], why: null, note: /any distribution/ },
  { name: 'Linux Chrome without a distribution name', env: { ua: UA.linuxChrome, platform: 'Linux x86_64', touch: 0, uaData: LIN_CH, hints: { architecture: 'x86' } },
    panel: 'linux', file: FILES.deb, label: 'Download for Linux (.deb)', sure: true, rec: [FILES.deb], why: null },
  { name: 'Linux on ARM', env: { ua: UA.linuxArmFirefox, platform: 'Linux aarch64', touch: 0 },
    panel: 'linux', file: null, label: 'See the Linux downloads', sure: true, rec: [], why: /no ARM build yet/, note: /No build for ARM Linux/ },
  { name: 'iPhone', env: { ua: UA.iphone, platform: 'iPhone', touch: 5 },
    panel: null, file: null, label: 'Get oneMK for your computer', sure: false, rec: [], mobile: true, note: /desktop program/ },
  { name: 'Android phone', env: { ua: UA.pixel7, platform: 'Linux armv81', touch: 5, uaData: { platform: 'Android', mobile: true } },
    panel: null, file: null, label: 'Get oneMK for your computer', sure: false, rec: [], mobile: true },
  { name: 'iPad asking for the desktop site', env: { ua: UA.macSafari, platform: 'MacIntel', touch: 5 },
    panel: null, file: null, label: 'Get oneMK for your computer', sure: false, rec: [], mobile: true },
  { name: 'ChromeOS', env: { ua: UA.chromeOS, platform: 'Linux x86_64', touch: 0, uaData: { platform: 'Chrome OS', mobile: false } },
    panel: null, file: null, label: 'Get oneMK for your computer', sure: false, rec: [], mobile: true },
  { name: 'A crawler that says nothing', env: { ua: 'Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)', platform: '', touch: 0 },
    panel: null, file: null, label: 'Get oneMK for your computer', sure: false, rec: [], mobile: false, note: /For Linux, macOS and Windows/ },
];

const PANEL_FILES: Record<string, string[]> = {
  mac: [FILES.macArm, FILES.macIntel],
  windows: [FILES.winSetup, FILES.winZip],
  linux: [FILES.deb, FILES.appImage, FILES.tarLinux],
};

async function read(browser: import('@e2e-dev/web').Browser) {
  return browser.evaluate(() => {
    const vis = (e: Element | null) => !!e && !(e as HTMLElement).hidden && getComputedStyle(e).display !== 'none' && (e as HTMLElement).offsetWidth > 0;
    const primaries = Array.from(document.querySelectorAll('a[data-download]')).map((a) => ({
      href: (a as HTMLAnchorElement).getAttribute('href') ?? '',
      label: (a.querySelector('[data-download-label]')?.textContent ?? '').trim(),
    }));
    const note = (document.querySelector('[data-download-note]')?.textContent ?? '').replace(/\s+/g, ' ').trim();
    const cards = Array.from(document.querySelectorAll('[data-panel]')).map((c) => {
      const el = c as HTMLElement;
      const r = el.getBoundingClientRect();
      return {
        panel: el.dataset.panel ?? '',
        detected: el.classList.contains('is-detected'),
        badge: vis(el.querySelector('[data-badge]')),
        why: vis(el.querySelector('[data-why]')) ? (el.querySelector('[data-why]')?.textContent ?? '').trim() : '',
        rec: Array.from(el.querySelectorAll('a.is-rec')).map((a) => (a as HTMLElement).dataset.file ?? ''),
        files: Array.from(el.querySelectorAll('a[data-file]')).map((a) => (a as HTMLElement).dataset.file ?? ''),
        top: Math.round(r.top + window.scrollY), left: Math.round(r.left), right: Math.round(r.right), width: Math.round(r.width),
      };
    });
    const grid = document.querySelector('[data-dl-grid]') as HTMLElement;
    return {
      primaries, note, cards,
      hasDetected: grid.classList.contains('has-detected'),
      away: vis(document.querySelector('[data-dl-away]')),
      detectedAttr: document.documentElement.dataset.detected ?? '',
      vw: document.documentElement.clientWidth,
    };
  });
}

function check(c: Case, got: Awaited<ReturnType<typeof read>>) {
  const href = c.file ? RELEASE + c.file : '#download';
  // Both big buttons, the hero's and the closing call's, agree.
  expect(got.primaries.length).toBe(2);
  for (const p of got.primaries) { expect(p.href, 'button target').toBe(href); expect(p.label, 'button label').toBe(c.label); }
  if (c.note) expect(got.note).toMatch(c.note);
  expect(got.note).toMatch(/^Version \d+\.\d+\.\d+/);
  // Exactly the right card is marked, and only it shows a badge and a reason.
  expect(got.cards.map((k) => k.panel).sort()).toEqual(['linux', 'mac', 'windows']);
  for (const card of got.cards) {
    const on = card.panel === c.panel;
    expect(card.detected, `${card.panel} card detected`).toBe(on);
    expect(card.badge, `${card.panel} badge`).toBe(on);
    if (on && c.why !== undefined && c.why !== null) expect(card.why, `${card.panel} reason`).toMatch(c.why);
    if (!on || c.why === null) expect(card.why, `${card.panel} reason`).toBe('');
    expect(card.rec, `${card.panel} recommended file`).toEqual(on ? c.rec : []);
  }
  expect(got.hasDetected).toBe(c.panel !== null);
  expect(got.away).toBe(!!c.mobile);
  // The detected card comes first in reading order (top of the grid, or leftmost on the top row).
  if (c.panel) {
    const mine = got.cards.find((k) => k.panel === c.panel)!;
    for (const other of got.cards) if (other !== mine) {
      expect(mine.top < other.top || (mine.top === other.top && mine.left < other.left), `${c.panel} card should lead`).toBe(true);
    }
  }
  const [os, , sure] = got.detectedAttr.split(' ');
  expect(sure).toBe(c.sure ? 'sure' : 'guess');
  expect(os).toBeTruthy();
}

for (const c of CASES.filter((x) => !x.gl)) {
  test(`download chooser: ${c.name}`, async ({ app, browser, guard }) => {
    await pretend(browser, c.env);
    await app.open('/');
    await detected(browser);
    check(c, await read(browser));
    await guard.check();
  });
}

// The graphics-chip guess needs a real WebGL context to ask; motion is reduced so no 3D scene runs.
for (const c of CASES.filter((x) => x.gl)) {
  test3d(`download chooser: ${c.name}`, async ({ app, browser, guard }) => {
    await browser.emulateMedia({ reducedMotion: 'reduce' });
    await pretend(browser, c.env);
    await app.open('/');
    // A browser with no graphics chip and no software renderer (the Firefox on a CI runner) has no WebGL to
    // ask: the page then has no hint to give, which another test covers.
    const webgl = await browser.evaluate<boolean>(`(function(){ try { var c = document.createElement('canvas'); return !!(c.getContext('webgl') || c.getContext('webgl2')); } catch (e) { return false; } })()`);
    test.skip(!webgl, 'this browser has no WebGL here, so there is no graphics chip to ask');
    await detected(browser);
    check(c, await read(browser));
    await guard.check();
  });
}

test('without scripts every platform is listed and the big button goes to the latest release', async ({ app, browser, guard }) => {
  await browser.route(/\/(js|vendor)\/[^/]+\.js$/, async (route) => { await route.fulfill({ status: 200, body: '', contentType: 'text/javascript' }); });
  await app.open('/');
  const got = await read(browser);
  for (const p of got.primaries) {
    expect(p.href).toBe('https://github.com/samer2373/one-mk/releases/latest');
    expect(p.label).toBe('Download oneMK');
  }
  expect(got.cards.every((k) => !k.detected && !k.badge && !k.rec.length)).toBe(true);
  expect(got.away).toBe(false);
  expect(got.cards.map((k) => k.files.length)).toEqual([2, 2, 3]);
  await guard.check();
});

test('every file link in the download cards is a current release asset, with the same name in the link, the label and the data', async ({ app, browser, guard }) => {
  await app.open('/');
  const links = await browser.evaluate(() => Array.from(document.querySelectorAll('a[href*="/releases/"]')).map((a) => ({
    href: (a as HTMLAnchorElement).href, file: (a as HTMLElement).dataset.file ?? '', shown: (a.querySelector('.asset__file')?.textContent ?? '').trim(),
    inCard: !!a.closest('[data-panel]'), text: (a.textContent ?? '').replace(/\s+/g, ' ').trim(),
  })));
  const yml = readFileSync(join(SITE_DIR, '..', '.github', 'workflows', 'release.yml'), 'utf8');
  const table = [...yml.matchAll(/^\s*\|[^|]*\|\s*`(one-mk-[^`]+)`\s*\|/gm)].map((m) => m[1]!);
  expect(table.sort()).toEqual([FILES.macArm, FILES.macIntel, FILES.winSetup, FILES.deb, FILES.appImage, FILES.tarLinux].sort());
  const targets = [...yml.matchAll(/target:\s*([a-z0-9_-]+)\s*\}/g)].map((m) => m[1]!);
  expect(targets.sort()).toEqual(['aarch64-apple-darwin', 'x86_64-apple-darwin', 'x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu']);
  const assets = new Set<string>(['SHA256SUMS', 'update-manifest.json', ...table, ...targets.map((t) => (t.includes('windows') ? `one-mk-${t}.zip` : `one-mk-${t}.tar.gz`))]);
  expect(yml).toContain('sha256sum * > SHA256SUMS');
  expect(yml).toContain('make-update-manifest');
  const seen = new Set<string>();
  for (const l of links) {
    if (!l.href.startsWith(RELEASE)) { expect(l.href).toBe('https://github.com/samer2373/one-mk/releases/latest'); continue; }
    const name = l.href.slice(RELEASE.length);
    expect(assets.has(name), `${name} is not a release asset`).toBe(true);
    seen.add(name);
    if (l.file) { expect(l.file).toBe(name); expect(l.shown).toBe(name); }
  }
  // Everything the release advertises as a download is on the page, and the page offers every build.
  for (const f of table) expect(seen.has(f), `${f} is not linked from the page`).toBe(true);
  for (const t of targets) expect(seen.has(t.includes('windows') ? `one-mk-${t}.zip` : `one-mk-${t}.tar.gz`), `${t} archive not linked`).toBe(true);
  expect(seen.has('SHA256SUMS')).toBe(true);
  // The files in each card are the ones that platform needs.
  const perCard = await browser.evaluate(() => Object.fromEntries(Array.from(document.querySelectorAll('[data-panel]')).map((c) => [(c as HTMLElement).dataset.panel, Array.from(c.querySelectorAll('a[data-file]')).map((a) => (a as HTMLElement).dataset.file)])));
  expect(perCard).toEqual(PANEL_FILES);
  await guard.check();
});

test('download sizes are shown for every file and look like sizes', async ({ app, browser, guard }) => {
  await app.open('/');
  const sizes = await browser.evaluate(() => Array.from(document.querySelectorAll('[data-size]')).map((s) => ({ f: (s as HTMLElement).dataset.size ?? '', t: (s.textContent ?? '').trim() })));
  expect(sizes.length).toBeGreaterThanOrEqual(9);
  for (const s of sizes) expect(s.t, s.f).toMatch(/^\d+(\.\d)? (KB|MB)$/);
  await guard.check();
});

test('on a phone-like visitor the "copy the link" button puts this page’s address on the clipboard or shows it', async ({ app, browser, screen, guard }) => {
  await browser.addInitScript(`(function(){
    var orig = navigator.clipboard && navigator.clipboard.writeText ? navigator.clipboard.writeText.bind(navigator.clipboard) : null;
    window.__copied = [];
    try { Object.defineProperty(navigator, 'clipboard', { value: { writeText: function(t){ window.__copied.push(t); return Promise.resolve(); } }, configurable: true }); } catch (e) {}
    try { Object.defineProperty(navigator, 'share', { value: undefined, configurable: true }); } catch (e) {}
  })();`);
  await pretend(browser, { ua: UA.iphone, platform: 'iPhone', touch: 5 });
  await app.open('/');
  await detected(browser);
  await expect(screen.getByText('oneMK is a desktop program.')).toBeVisible();
  const button = screen.getByRole('button', { name: 'Copy the link' });
  await button.scrollIntoView();
  await button.tap();
  await expect(browser.locator('[data-share-done]')).toHaveText('Copied. Open it on your computer.');
  const copied = await browser.evaluate<string[]>('window.__copied');
  expect(copied).toEqual([`${app.baseUrl}/#download`]);
  await guard.check();
});
