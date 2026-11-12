import { test, expect, currentProfile } from '../lib/support.ts';

// "Set up in about a minute": four steps; on wide screens a sticky window beside them follows the scroll.

const state = (browser: import('@e2e-dev/web').Browser) => browser.evaluate(() => {
  const steps = Array.from(document.querySelectorAll('[data-step]'));
  const panes = Array.from(document.querySelectorAll('[data-pane]'));
  const view = document.querySelector('.how__view') as HTMLElement;
  const pics = steps.map((s) => {
    const vis = (e: Element | null) => !!e && getComputedStyle(e).display !== 'none' && (e as HTMLElement).offsetWidth > 0;
    const shown = Array.from(s.querySelectorAll('.step__pic picture img')).find((i) => vis(i)) as HTMLImageElement | undefined;
    return shown ? { loaded: shown.complete && shown.naturalWidth > 0, right: Math.round(shown.getBoundingClientRect().right) } : null;
  });
  return {
    onStep: steps.map((s) => s.classList.contains('is-on')),
    onPane: panes.map((p) => p.classList.contains('is-on')),
    viewShown: getComputedStyle(view).display !== 'none' && view.offsetWidth > 0,
    pics, vw: document.documentElement.clientWidth,
    sticky: getComputedStyle(document.querySelector('.how__sticky') as HTMLElement).position,
  };
});

test('four steps in order, each with its heading, text and command or picture', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const how = screen.getByRole('heading', { name: 'Set up in about a minute.' });
  await expect(how).toBeVisible();
  const r = await browser.evaluate(() => Array.from(document.querySelectorAll('.how [data-step]')).map((s) => ({
    n: (s as HTMLElement).dataset.step ?? '', h: (s.querySelector('h3')?.textContent ?? '').trim(), cmd: (s.querySelector('.step__cmd code')?.textContent ?? '').trim(), hasPic: !!s.querySelector('.step__pic picture'),
  })));
  expect(r.map((s) => s.h)).toEqual(['Pair with six digits', 'Arrange your screens', 'Push past the edge', 'Everything follows']);
  expect(r.map((s) => s.n)).toEqual(['0', '1', '2', '3']);
  expect(r.map((s) => s.cmd)).toEqual(['one-mk pair', 'one-mk layout "MacBook Air" left', '', 'one-mk send photos/ --to "MacBook Air"']);
  expect(r.every((s) => s.hasPic)).toBe(true);
  await guard.check();
});

test('the story follows the scroll: the step in the middle of the screen is lit and its picture shows', async ({ app, browser, guard }) => {
  await app.open('/');
  const p = await currentProfile(browser);
  const s0 = await state(browser);
  expect(s0.onStep.filter(Boolean).length).toBe(1);
  expect(s0.onPane.filter(Boolean).length).toBe(1);
  for (const i of [1, 2, 3, 2, 0, 3]) {
    await browser.evaluate((k: number) => { document.querySelector(`[data-step="${k}"]`)!.scrollIntoView({ block: 'center', behavior: 'instant' as ScrollBehavior }); return true; }, i);
    await expect.poll(async () => (await state(browser)).onStep.indexOf(true), { timeout: 8000 }).toBe(i);
    const s = await state(browser);
    expect(s.onStep.filter(Boolean).length, 'one step lit').toBe(1);
    expect(s.onPane.filter(Boolean).length, 'one pane lit').toBe(1);
    expect(s.onPane.indexOf(true), 'pane matches step').toBe(i);
  }
  // The sticky window beside the steps exists only on wide screens; below that each step carries its own picture.
  const s = await state(browser);
  if (p.viewport.width >= 1024) {
    expect(s.viewShown).toBe(true);
    expect(s.sticky).toBe('sticky');
  } else {
    expect(s.viewShown).toBe(false);
  }
  await guard.check();
});

test('every step has a picture that loads and fits the screen (the sticky window on wide screens, the step’s own below)', async ({ app, browser, guard }) => {
  await app.open('/');
  const p = await currentProfile(browser);
  for (const i of [0, 1, 2, 3]) {
    await browser.evaluate((k: number) => { document.querySelector(`[data-step="${k}"]`)!.scrollIntoView({ block: 'center', behavior: 'instant' as ScrollBehavior }); return true; }, i);
    await expect.poll(async () => (await state(browser)).onStep.indexOf(true), { timeout: 8000 }).toBe(i);
    if (p.viewport.width >= 1024) {
      await expect.poll(() => browser.evaluate((k: number) => {
        const imgs = Array.from(document.querySelectorAll(`[data-pane="${k}"] picture img`)) as HTMLImageElement[];
        const shown = imgs.find((x) => x.offsetWidth > 0);
        return !!shown && shown.complete && shown.naturalWidth > 0 && shown.getBoundingClientRect().right <= document.documentElement.clientWidth;
      }, i), { timeout: 15_000 }).toBe(true);
    } else {
      await expect.poll(async () => (await state(browser)).pics[i]?.loaded ?? false, { timeout: 15_000 }).toBe(true);
      const pic = (await state(browser)).pics[i]!;
      expect(pic.right).toBeLessThanOrEqual((await state(browser)).vw);
    }
  }
  await guard.check();
});
