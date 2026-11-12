import { test, expect, currentProfile } from '../lib/support.ts';

// The window tour: seven tabs with arrow-key navigation, each showing its own screenshot.

const TABS = [
  ['t-desk', 'p-desk', 'Desk'],
  ['t-arrange', 'p-arrange', 'Arrange'],
  ['t-wallpaper', 'p-wallpaper', 'Wallpaper'],
  ['t-files', 'p-files', 'Clipboard & files'],
  ['t-keys', 'p-keys', 'Keys & mouse'],
  ['t-health', 'p-health', 'Health'],
  ['t-settings', 'p-settings', 'Settings'],
] as const;

async function snapshot(browser: import('@e2e-dev/web').Browser) {
  return browser.evaluate(() => {
    const list = document.querySelector('[data-tabs]') as HTMLElement;
    const lr = list.getBoundingClientRect();
    return {
      tabs: Array.from(list.querySelectorAll('[role="tab"]')).map((t) => {
        const r = t.getBoundingClientRect();
        return { id: t.id, selected: t.getAttribute('aria-selected'), tabindex: (t as HTMLElement).tabIndex, controls: t.getAttribute('aria-controls'),
          inStrip: r.left >= lr.left - 1 && r.right <= lr.right + 1 };
      }),
      panels: Array.from(document.querySelectorAll('[role="tabpanel"]')).map((p) => {
        const img = p.querySelector('picture img') as HTMLImageElement | null;
        const r = p.getBoundingClientRect();
        return { id: p.id, hidden: (p as HTMLElement).hidden, labelledby: p.getAttribute('aria-labelledby'), shown: (p as HTMLElement).offsetParent !== null,
          img: img ? { complete: img.complete, w: img.naturalWidth, vis: img.offsetWidth > 0, right: Math.round(img.getBoundingClientRect().right) } : null,
          cap: (p.querySelector('.device__cap')?.textContent ?? '').trim(), w: Math.round(r.width) };
      }),
      active: document.activeElement?.id ?? '',
      vw: document.documentElement.clientWidth,
    };
  });
}

test('the tour starts on Desk, with one tab in the tab order and one panel showing', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const s = await snapshot(browser);
  expect(s.tabs.map((t) => t.id)).toEqual(TABS.map((t) => t[0]));
  expect(s.tabs.map((t) => t.selected)).toEqual(['true', 'false', 'false', 'false', 'false', 'false', 'false']);
  expect(s.tabs.map((t) => t.tabindex)).toEqual([0, -1, -1, -1, -1, -1, -1]);
  expect(s.panels.map((p) => p.shown)).toEqual([true, false, false, false, false, false, false]);
  for (const [i, [tab, panel]] of TABS.entries()) {
    expect(s.tabs[i]!.controls).toBe(panel);
    expect(s.panels[i]!.id).toBe(panel);
    expect(s.panels[i]!.labelledby).toBe(tab);
  }
  await expect(screen.getByRole('tablist', { name: 'Pages of the oneMK window' })).toBeVisible();
  await guard.check();
});

test('choosing each tab shows exactly its panel and a loaded screenshot that fits the screen', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const tablist = screen.getByRole('tablist', { name: 'Pages of the oneMK window' });
  await tablist.scrollIntoView();
  for (const [i, [tab, , label]] of TABS.entries()) {
    await tablist.getByRole('tab', { name: label }).tap();
    await expect(tablist.getByRole('tab', { name: label })).toHaveAttribute('aria-selected', 'true');
    await expect.poll(async () => { const p = (await snapshot(browser)).panels[i]!; return p.img ? p.img.complete && p.img.w > 0 : false; }, { timeout: 15_000 }).toBe(true);
    const s = await snapshot(browser);
    expect(s.panels.map((p) => p.shown), `only ${label} shows`).toEqual(TABS.map((_, k) => k === i));
    expect(s.tabs.map((t) => t.selected)).toEqual(TABS.map((_, k) => (k === i ? 'true' : 'false')));
    expect(s.tabs.map((t) => t.tabindex)).toEqual(TABS.map((_, k) => (k === i ? 0 : -1)));
    expect(s.tabs[i]!.inStrip, `${label} tab is inside the strip`).toBe(true);
    const p = s.panels[i]!;
    expect(p.img!.vis).toBe(true);
    expect(p.img!.right, `${label} screenshot inside the screen`).toBeLessThanOrEqual(s.vw);
    expect(p.cap.toLowerCase().startsWith(label.toLowerCase().split(' ')[0]!.toLowerCase()), `caption "${p.cap}"`).toBe(true);
  }
  await guard.check();
});

test('arrow keys, Home and End move between tabs, wrap around, and move focus with the selection', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const first = screen.getByRole('tab', { name: 'Desk' });
  await first.scrollIntoView();
  await first.focus();
  const order = TABS.map((t) => t[0]);
  const now = async () => (await snapshot(browser));
  await browser.keyboard.press('ArrowRight');
  let s = await now();
  expect([s.active, s.tabs[1]!.selected, s.panels[1]!.shown, s.panels[0]!.shown]).toEqual(['t-arrange', 'true', true, false]);
  await browser.keyboard.press('End');
  s = await now();
  expect([s.active, s.tabs[6]!.selected]).toEqual(['t-settings', 'true']);
  await browser.keyboard.press('ArrowRight');
  s = await now();
  expect(s.active, 'wraps from the last to the first').toBe(order[0]);
  await browser.keyboard.press('ArrowLeft');
  s = await now();
  expect(s.active, 'wraps from the first to the last').toBe(order[6]);
  await browser.keyboard.press('Home');
  s = await now();
  expect(s.active).toBe(order[0]);
  // Down and Up do nothing (the tabs are a horizontal list); the page does not lose its place.
  await browser.keyboard.press('ArrowDown');
  s = await now();
  expect(s.tabs[0]!.selected).toBe('true');
  // One Tab leaves the list into the panel's content, it does not walk through every tab.
  await browser.keyboard.press('Tab');
  s = await now();
  expect(order).not.toContain(s.active);
  await guard.check();
});

test('the strip of tabs scrolls inside itself on narrow screens and never makes the page scroll sideways', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const p = await currentProfile(browser);
  const tablist = screen.getByRole('tablist', { name: 'Pages of the oneMK window' });
  await tablist.scrollIntoView();
  await tablist.getByRole('tab', { name: 'Settings' }).tap();
  const geo = await browser.evaluate(() => {
    const l = document.querySelector('[data-tabs]') as HTMLElement;
    return { scrollW: l.scrollWidth, clientW: l.clientWidth, left: l.scrollLeft, pageOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth };
  });
  expect(geo.pageOverflow).toBeLessThanOrEqual(0);
  if (p.viewport.width < 700) {
    expect(geo.scrollW).toBeGreaterThan(geo.clientW);
    expect(geo.left, 'the chosen tab was brought into view').toBeGreaterThan(0);
  }
  await guard.check();
});
