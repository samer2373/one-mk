import { test as base, type Browser } from '@e2e-dev/web';
import { expect } from 'e2e';
import { readFileSync } from 'node:fs';
import { dirname, join, extname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { profileByName, envScript, type EnvSpec, type Profile } from './profiles.ts';

export { expect };

export const SITE_DIR = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'site');

/** Collects, inside the page, everything that should never happen on a clean page. */
const WATCH_SCRIPT = `(function(){
  var KEY = '__e2eErrors', mem = [];
  function add(kind, msg){
    var line = kind + ': ' + msg; mem.push(line);
    try { var a = JSON.parse(sessionStorage.getItem(KEY) || '[]'); a.push(line); sessionStorage.setItem(KEY, JSON.stringify(a)); } catch (e) {}
  }
  window.__e2eMem = mem;
  window.addEventListener('error', function(e){
    var t = e.target;
    if (t && t !== window && t.tagName) add('resource-error', t.tagName.toLowerCase() + ' ' + (t.currentSrc || t.src || t.href || ''));
    else add('exception', (e.message || '') + ' @' + (e.filename || '') + ':' + (e.lineno || ''));
  }, true);
  window.addEventListener('unhandledrejection', function(e){ add('unhandled-rejection', String(e.reason && (e.reason.stack || e.reason))); });
  document.addEventListener('securitypolicyviolation', function(e){ add('csp', e.violatedDirective + ' ' + e.blockedURI); });
  ['error', 'warn'].forEach(function(k){
    var orig = console[k];
    console[k] = function(){ add('console.' + k, Array.prototype.slice.call(arguments).map(String).join(' ')); return orig.apply(console, arguments); };
  });
})();`;

const MIME: Record<string, string> = {
  '.html': 'text/html; charset=utf-8', '.css': 'text/css', '.js': 'text/javascript', '.woff2': 'font/woff2',
  '.svg': 'image/svg+xml', '.png': 'image/png', '.webp': 'image/webp', '.avif': 'image/avif', '.ico': 'image/x-icon',
  '.json': 'application/json', '.webmanifest': 'application/manifest+json',
};

export interface Guard {
  /** Every URL the page asked for. */
  readonly requests: string[];
  /** URLs on hosts that are not the site's own (they are aborted, never fetched). */
  readonly foreign: string[];
  /** Let a host through the "no requests to other hosts" rule. */
  allowHost(host: string): void;
  /** Serve every request to `origin` from the site folder (for 404.html, which links by absolute address). */
  mirror(origin: string): void;
  /** Stop the end-of-test check (for a test that provokes errors on purpose). */
  expectErrors(): void;
  /** Assert now: nothing failed, nothing foreign, every request answered below 400. Resets the page's error log. */
  check(): Promise<void>;
}

const statusCache = new Map<string, Promise<number>>();
/** HTTP status of a URL on the test server (0 when it did not answer in 40 s). Cached for the process. */
export function statusOf(url: string): Promise<number> {
  let p = statusCache.get(url);
  if (!p) {
    p = fetch(url, { signal: AbortSignal.timeout(40_000) }).then(async (r) => { await r.arrayBuffer(); return r.status; }, () => 0);
    statusCache.set(url, p);
  }
  return p;
}

/** Statuses of many URLs, a few at a time. */
export async function statuses(urls: string[], concurrency = 6): Promise<Record<string, number>> {
  const out: Record<string, number> = {};
  let i = 0;
  await Promise.all(Array.from({ length: concurrency }, async () => {
    while (i < urls.length) { const u = urls[i++]!; out[u] = await statusOf(u); }
  }));
  return out;
}

/** What a visitor without WebGL (or a blocklisted GPU) has: no context from any canvas. */
export const NO_WEBGL_SCRIPT = `(function(){
  var o = HTMLCanvasElement.prototype.getContext;
  HTMLCanvasElement.prototype.getContext = function(t){ return /webgl/i.test(String(t)) ? null : o.apply(this, arguments); };
  try { Object.defineProperty(window, 'WebGLRenderingContext', { value: undefined, configurable: true }); } catch (e) {}
  try { Object.defineProperty(window, 'WebGL2RenderingContext', { value: undefined, configurable: true }); } catch (e) {}
})();`;

function makeTest(webgl: boolean) {
  return base.extend<{ guard: Guard }>({
  guard: async ({ app, browser }, use) => {
    if (!webgl) await browser.addInitScript(NO_WEBGL_SCRIPT);
    const origin = new URL(app.baseUrl).origin;
    const allowed = new Set<string>([new URL(origin).host]);
    const mirrored: string[] = [];
    const requests: string[] = [];
    const foreign: string[] = [];
    let expectErrors = false;
    await browser.addInitScript(WATCH_SCRIPT);
    await browser.route(/.*/, async (route) => {
      const url = route.request.url;
      requests.push(url);
      const u = new URL(url);
      const mirror = mirrored.find((m) => url.startsWith(m));
      if (mirror) {
        const rel = decodeURIComponent(u.pathname.replace(new URL(mirror).pathname, '')) || 'index.html';
        try {
          const body = readFileSync(join(SITE_DIR, rel.endsWith('/') ? rel + 'index.html' : rel));
          await route.fulfill({ status: 200, body, contentType: MIME[extname(rel)] ?? 'application/octet-stream' });
        } catch { await route.fulfill({ status: 404, body: 'not found' }); }
        return;
      }
      if (!allowed.has(u.host)) { foreign.push(url); await route.abort(); return; }
      await route.continue();
    });
    const guard: Guard = {
      requests, foreign,
      allowHost: (h) => { allowed.add(h); },
      mirror: (o) => { mirrored.push(o); },
      expectErrors: () => { expectErrors = true; },
      async check() {
        const logged = await browser.evaluate<string[]>(
          `(function(){ try { var a = JSON.parse(sessionStorage.getItem('__e2eErrors') || '[]'); sessionStorage.removeItem('__e2eErrors'); return a.concat((window.__e2eMem || []).filter(function(l){ return a.indexOf(l) < 0; })); } catch (e) { return window.__e2eMem || []; } })()`,
        ).catch(() => [] as string[]);
        const bad: string[] = [];
        if (!expectErrors) bad.push(...logged);
        bad.push(...foreign.map((u) => 'request to another host: ' + u));
        const own = [...new Set(requests.filter((u) => u.startsWith(origin)))];
        const statuses = await Promise.all(own.map(async (u) => [u, await statusOf(u)] as const));
        for (const [u, s] of statuses) if (s === 0 || s >= 400) bad.push(`HTTP ${s} for ${u}`);
        if (bad.length) throw new Error('The page was not clean:\n  ' + bad.join('\n  '));
      },
    };
    await use(guard);
    if (!expectErrors) await guard.check();
  },
  });
}

/**
 * The default test: the page as a visitor without WebGL sees it (posters, no
 * three.js). Software WebGL on a CI machine is slow enough to starve the page's
 * own thread, so only the tests about the 3D desk turn it on.
 */
export const test = makeTest(false);
/** The same, with the browser's real (software) WebGL. */
export const test3d = makeTest(true);

export { base };

/** The profile this test runs on (read from the page, which the config's init script stamped). */
export async function currentProfile(browser: Browser): Promise<Profile> {
  const name = await browser.evaluate<string>('window.__e2eProfile || ""');
  return profileByName(name);
}

/** Add an init script that makes the page look like another computer. Call before app.open. */
export async function pretend(browser: Browser, env: EnvSpec): Promise<void> {
  await browser.addInitScript(envScript(env));
}

/** Wait until site.js has finished its download detection. */
export async function detected(browser: Browser): Promise<void> {
  await expect.poll(() => browser.evaluate<string>('document.documentElement.dataset.detectDone || ""'), { timeout: 10_000 }).toBe('1');
}

/**
 * Scroll the page top to bottom the way a visitor reads it, so lazy images and
 * scroll observers fire. At each stop it waits for the images shown there. It
 * ends at the bottom (call `scrollToTop` to go back).
 */
export async function scrollThrough(browser: Browser): Promise<void> {
  await browser.evaluate(async () => {
    const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
    const pending = () => Array.from(document.images).filter((i) => {
      if (i.complete || i.offsetParent === null) return false;
      const r = i.getBoundingClientRect();
      return r.bottom > -200 && r.top < window.innerHeight + 200;
    });
    const step = Math.max(300, Math.round(window.innerHeight * 0.7));
    for (let y = 0; y < document.documentElement.scrollHeight; y += step) {
      window.scrollTo(0, y);
      await wait(80);
      for (let n = 0; n < 40 && pending().length; n++) await wait(50);
    }
    window.scrollTo(0, document.documentElement.scrollHeight);
    await wait(1200);
    return true;
  });
}

export async function scrollToTop(browser: Browser): Promise<void> {
  await browser.evaluate(async () => { window.scrollTo(0, 0); await new Promise((r) => setTimeout(r, 150)); return true; });
}
