import { test, test3d, expect, currentProfile } from '../lib/support.ts';

// The 3D desk and keyboard, and what a visitor gets without WebGL or with reduced motion.

type B = import('@e2e-dev/web').Browser;

const hasWebGL = (browser: B) => browser.evaluate<boolean>(`(function(){ try { var c = document.createElement('canvas'); return !!(window.WebGLRenderingContext && (c.getContext('webgl2') || c.getContext('webgl'))); } catch (e) { return false; } })()`);

const heroState = (browser: B) => browser.evaluate(() => {
  const canvas = document.querySelector('[data-desk]') as HTMLCanvasElement;
  const frame = canvas.parentElement as HTMLElement;
  const fr = frame.getBoundingClientRect(), cr = canvas.getBoundingClientRect();
  const poster = document.querySelector('.stage__poster') as HTMLElement;
  const controls = document.querySelector('.stage__controls') as HTMLElement;
  const chips = Array.from(document.querySelectorAll('[data-go]')) as HTMLElement[];
  const labels = Array.from(document.querySelectorAll('[data-labels] li')) as HTMLElement[];
  return {
    live: canvas.classList.contains('is-live'),
    three: typeof (window as unknown as { THREE?: unknown }).THREE !== 'undefined',
    desk: typeof (window as unknown as { OMKDesk?: unknown }).OMKDesk !== 'undefined',
    canvasPx: [canvas.width, canvas.height], canvasBox: [Math.round(cr.width), Math.round(cr.height)], frameBox: [Math.round(fr.width), Math.round(fr.height)],
    posterOpacity: getComputedStyle(poster).opacity,
    controlsShown: !controls.hidden && getComputedStyle(controls).display !== 'none',
    pressed: chips.map((c) => c.getAttribute('aria-pressed')),
    chipNames: chips.map((c) => (c.textContent ?? '').trim()),
    labels: labels.map((l) => ({ text: (l.textContent ?? '').trim(), on: l.classList.contains('is-on') })),
    labelsLive: (document.querySelector('[data-labels]') as HTMLElement).classList.contains('is-live'),
    hint: (document.querySelector('[data-stage-hint]')?.textContent ?? '').trim(),
  };
});

/** Software WebGL at 2560 px is too slow to drive interactively, so the interaction tests skip that one profile. */
async function interactive(browser: B) {
  const p = await currentProfile(browser);
  test.skip(p.viewport.width > 2000, 'software WebGL at this size is too slow to drive interactively; the load test still runs');
  test.skip(!(await hasWebGL(browser)), 'this browser has no WebGL here');
}

test3d('with WebGL the hero desk loads, fades the poster out, and shows the three computers', async ({ app, browser, guard }) => {
  await app.open('/');
  test.skip(!(await hasWebGL(browser)), 'this browser has no WebGL here');
  await expect.poll(async () => (await heroState(browser)).live, { timeout: 100_000, message: 'the canvas never went live' }).toBe(true);
  const s = await heroState(browser);
  expect(s.three).toBe(true);
  expect(s.desk).toBe(true);
  expect(s.controlsShown).toBe(true);
  expect(s.chipNames).toEqual(['MacBook Air', 'Studio PC', 'Gaming PC']);
  expect(s.pressed).toEqual(['false', 'true', 'false']);
  expect(s.labelsLive).toBe(true);
  expect(s.labels.map((l) => l.text.replace(/\s+/g, ' '))).toEqual(['MacBook Air macOS', 'Studio PC Linux', 'Gaming PC Windows']);
  // The canvas covers its frame exactly and draws at least that many pixels.
  expect(s.canvasBox).toEqual(s.frameBox);
  expect(s.canvasPx[0]).toBeGreaterThanOrEqual(Math.floor(s.frameBox[0]! * 0.6));
  await expect.poll(async () => +(await heroState(browser)).posterOpacity, { timeout: 5000 }).toBeLessThan(0.05);
  // three.js and the scene code came from this site only.
  expect(guard.requests.filter((u) => /three\.min\.js|desk\.js|mouse-mesh\.js/.test(u)).length).toBe(3);
  await guard.check();
});

test3d('the buttons under the desk move the pointer to that computer, with mouse and keyboard', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  await interactive(browser);
  await expect.poll(async () => (await heroState(browser)).live, { timeout: 90_000 }).toBe(true);
  const group = screen.getByRole('group', { name: 'Move the pointer to a computer' });
  const mac = group.getByRole('button', { name: 'MacBook Air' });
  const pc = group.getByRole('button', { name: 'Gaming PC' });
  const studio = group.getByRole('button', { name: 'Studio PC' });
  await mac.tap();
  await expect.poll(async () => (await heroState(browser)).pressed, { timeout: 60_000 }).toEqual(['true', 'false', 'false']);
  await expect.poll(async () => (await heroState(browser)).labels.map((l) => l.on), { timeout: 5000 }).toEqual([true, false, false]);
  await pc.tap();
  await expect.poll(async () => (await heroState(browser)).pressed, { timeout: 60_000 }).toEqual(['false', 'false', 'true']);
  await expect.poll(async () => (await heroState(browser)).labels.map((l) => l.on), { timeout: 5000 }).toEqual([false, false, true]);
  // From the keyboard.
  await studio.focus();
  await browser.keyboard.press('Enter');
  await expect.poll(async () => (await heroState(browser)).pressed, { timeout: 60_000 }).toEqual(['false', 'true', 'false']);
  await mac.focus();
  await browser.keyboard.press('Space');
  await expect.poll(async () => (await heroState(browser)).pressed, { timeout: 60_000 }).toEqual(['true', 'false', 'false']);
  await guard.check();
});

test3d('moving the mouse over the desk drives the pointer and the hint says so (not on touch)', async ({ app, browser, guard }) => {
  await app.open('/');
  const p = await currentProfile(browser);
  await interactive(browser);
  await expect.poll(async () => (await heroState(browser)).live, { timeout: 90_000 }).toBe(true);
  await browser.evaluate("document.querySelector('[data-desk]').scrollIntoView({ block: 'center', behavior: 'instant' })");
  await browser.evaluate('new Promise(function (r) { setTimeout(r, 300); })');
  const box = await browser.evaluate(() => { const r = (document.querySelector('[data-desk]') as HTMLElement).getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height }; });
  expect((await heroState(browser)).hint).toBe('Move your mouse across the desk.');
  await browser.mouse.move(box.x + box.w * 0.15, box.y + box.h * 0.4);
  await browser.mouse.move(box.x + box.w * 0.3, box.y + box.h * 0.45);
  await expect.poll(async () => (await heroState(browser)).hint, { timeout: 30_000 }).toBe('The pointer crosses at the same height, like oneMK does.');
  expect(p.name).toBeTruthy();
  await guard.check();
});

test3d('the Keys section loads its own 3D keyboard and the four quick setups change the caption and the pressed button', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  await interactive(browser);
  await browser.evaluate("document.querySelector('#keys').scrollIntoView({ block: 'center', behavior: 'instant' })");
  await expect.poll(() => browser.evaluate<boolean>("document.querySelector('[data-keys]').classList.contains('is-live')"), { timeout: 90_000 }).toBe(true);
  const status = () => browser.evaluate<string>("document.querySelector('[data-remap-status]').textContent.trim()");
  const pressed = () => browser.evaluate<string[]>("Array.from(document.querySelectorAll('[data-remap]')).map(function (b) { return b.getAttribute('aria-pressed'); })");
  expect(await status()).toBe('Caps Lock: tap for Esc, hold for Ctrl.');
  const group = screen.getByRole('group', { name: 'Try a quick setup' });
  const cases: [string, string, string[]][] = [
    ['Swap Alt and Super', 'Alt and Super trade places.', ['false', 'true', 'false', 'false']],
    ['Swap Ctrl and Cmd', 'Ctrl and Cmd trade places.', ['false', 'false', 'true', 'false']],
    ['Arrows on Alt + HJKL', 'Hold Alt: H, J, K and L move like the arrow keys.', ['false', 'false', 'false', 'true']],
    ['Caps Lock as Esc', 'Caps Lock: tap for Esc, hold for Ctrl.', ['true', 'false', 'false', 'false']],
  ];
  for (const [name, text, state] of cases) {
    await group.getByRole('button', { name }).tap();
    expect(await status()).toBe(text);
    expect(await pressed()).toEqual(state);
  }
  // Keyboard: Space on a quick setup applies it.
  await group.getByRole('button', { name: 'Swap Ctrl and Cmd' }).focus();
  await browser.keyboard.press('Space');
  expect(await pressed()).toEqual(['false', 'false', 'true', 'false']);
  await guard.check();
});

test3d('switching the theme while the 3D scenes are live does not break them', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  await interactive(browser);
  await expect.poll(async () => (await heroState(browser)).live, { timeout: 90_000 }).toBe(true);
  const before = await browser.evaluate<string>('document.documentElement.dataset.theme');
  await screen.getByRole('button', { name: /Switch to (dark|light) theme/ }).tap();
  await expect.poll(() => browser.evaluate<string>('document.documentElement.dataset.theme')).not.toBe(before);
  await browser.evaluate('new Promise(function (r) { setTimeout(r, 800); })');
  expect((await heroState(browser)).live).toBe(true);
  await guard.check();
});

// ---------------------------------------------------------------- fallbacks --

for (const mode of ['no-webgl', 'reduced-motion'] as const) {
  const t = mode === 'no-webgl' ? test : test3d;
  t(`${mode}: the poster stays, the controls that need 3D are gone, three.js is never fetched, and nothing errors`, async ({ app, browser, guard }) => {
    if (mode === 'reduced-motion') await browser.emulateMedia({ reducedMotion: 'reduce' });
    await app.open('/');
    const shown = (sel: string) => browser.evaluate<boolean>(`(function(){ var i = Array.from(document.querySelectorAll('${sel}')).find(function (x) { return x.offsetWidth > 0; }); return !!i && i.complete && i.naturalWidth > 0; })()`);
    await expect.poll(() => shown('.stage__poster img'), { timeout: 20_000 }).toBe(true);
    await browser.evaluate("document.querySelector('#keys').scrollIntoView({ block: 'center', behavior: 'instant' })");
    await expect.poll(() => shown('.remap__poster img'), { timeout: 20_000 }).toBe(true);
    await browser.evaluate('new Promise(function (r) { setTimeout(r, 800); })');
    const s = await browser.evaluate(() => {
      const vis = (e: Element | null) => !!e && getComputedStyle(e).display !== 'none' && (e as HTMLElement).offsetWidth > 0 && getComputedStyle(e).visibility !== 'hidden';
      const posterImg = Array.from(document.querySelectorAll('.stage__poster img')).find((i) => vis(i)) as HTMLImageElement | undefined;
      const keysImg = Array.from(document.querySelectorAll('.remap__poster img')).find((i) => vis(i)) as HTMLImageElement | undefined;
      const marquee = document.querySelector('[data-marquee]') as HTMLElement;
      return {
        three: typeof (window as unknown as { THREE?: unknown }).THREE !== 'undefined',
        live: document.querySelectorAll('canvas.is-live').length,
        controls: vis(document.querySelector('.stage__controls')),
        presets: vis(document.querySelector('.presets')),
        poster: !!posterImg && posterImg.complete && posterImg.naturalWidth > 0,
        posterOpacity: getComputedStyle(document.querySelector('.stage__poster') as HTMLElement).opacity,
        keysShot: !!keysImg && keysImg.complete && keysImg.naturalWidth > 0,
        keysShotOpacity: getComputedStyle(document.querySelector('.remap__poster') as HTMLElement).opacity,
        marqueeMoving: marquee.classList.contains('is-marquee'),
        marqueeAnim: getComputedStyle(marquee).animationName,
        status: (document.querySelector('[data-remap-status]')?.textContent ?? '').trim(),
      };
    });
    expect(s.three).toBe(false);
    expect(s.live).toBe(0);
    expect(s.controls, 'the chips need the 3D desk').toBe(false);
    expect(s.presets, 'the quick setups need the 3D keyboard').toBe(false);
    expect(s.poster).toBe(true);
    expect(+s.posterOpacity).toBe(1);
    expect(s.keysShot, 'a real screenshot of the Keys page stands in for the 3D keyboard').toBe(true);
    expect(+s.keysShotOpacity).toBe(1);
    if (mode === 'reduced-motion') {
      expect(s.marqueeMoving).toBe(false);
    }
    expect(guard.requests.filter((u) => /three\.min\.js|desk\.js|mouse-mesh\.js/.test(u))).toEqual([]);
    await guard.check();
  });
}

test('reduced motion: nothing waits for an entrance animation, so all text is visible without scrolling to it', async ({ app, browser, guard }) => {
  await browser.emulateMedia({ reducedMotion: 'reduce' });
  await app.open('/');
  const r = await browser.evaluate(() => {
    const hidden: string[] = [];
    document.querySelectorAll('[data-reveal], .hero__title span, .hero__lede, .hero__actions, .hero__meta').forEach((e) => {
      const cs = getComputedStyle(e);
      if (+cs.opacity < 0.99 || cs.transform !== 'none') hidden.push(`${(e as HTMLElement).className || e.tagName} opacity ${cs.opacity} transform ${cs.transform}`);
    });
    const anim = Array.from(document.querySelectorAll('*')).filter((e) => { const cs = getComputedStyle(e); return cs.animationName !== 'none' && parseFloat(cs.animationDuration) > 0.05; }).map((e) => `${e.tagName}.${(e as HTMLElement).className}`);
    return { hidden, anim, revealCount: document.querySelectorAll('[data-reveal]').length };
  });
  expect(r.revealCount).toBeGreaterThan(30);
  expect(r.hidden).toEqual([]);
  expect(r.anim, 'animations that still run under reduced motion').toEqual([]);
  await guard.check();
});

test('with motion allowed every entrance finishes: after scrolling past, no block is left faded out', async ({ app, browser, guard }) => {
  await app.open('/');
  const total = await browser.evaluate<number>("document.querySelectorAll('[data-reveal]').length");
  expect(total).toBeGreaterThan(30);
  for (let y = 0; ; y += 500) {
    const more = await browser.evaluate(async (yy: number) => { window.scrollTo(0, yy); await new Promise((r) => setTimeout(r, 120)); return yy < document.documentElement.scrollHeight; }, y);
    if (!more) break;
  }
  await browser.evaluate("document.querySelector('.foot').scrollIntoView({ block: 'end', behavior: 'instant' })");
  await browser.evaluate('new Promise(function (r) { setTimeout(r, 1200); })');
  // Look at every block as a visitor reading it would: bring it to the middle and wait for it to be fully drawn.
  const stuck = await browser.evaluate(async () => {
    const out: string[] = [];
    for (const e of Array.from(document.querySelectorAll('[data-reveal]'))) {
      if ((e as HTMLElement).offsetParent === null) continue; // not displayed (the other theme, a closed tab)
      e.scrollIntoView({ block: 'center', behavior: 'instant' as ScrollBehavior });
      let o = 0;
      for (let t = 0; t < 40; t++) { await new Promise((r) => setTimeout(r, 50)); o = +getComputedStyle(e).opacity; if (o >= 0.99) break; }
      if (o < 0.99) out.push(`${(e as HTMLElement).className || e.tagName}: opacity ${o}`);
    }
    return out;
  });
  expect(stuck).toEqual([]);
  await guard.check();
});
