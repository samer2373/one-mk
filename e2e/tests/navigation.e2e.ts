import { test, expect, currentProfile } from '../lib/support.ts';

// The top bar, in-page anchors, the skip link, the footer.

const NAV: [string, string][] = [
  ['How it works', '#how'],
  ['Features', '#features'],
  ['Security', '#security'],
  ['Download', '#download'],
  ['FAQ', '#faq'],
];

/** Wait until the page has stopped scrolling (smooth scrolling takes a moment). */
async function settled(browser: import('@e2e-dev/web').Browser) {
  let last = -1, same = 0;
  await expect.poll(async () => {
    const y = await browser.evaluate<number>('Math.round(window.scrollY)');
    same = y === last ? same + 1 : 0;
    last = y;
    return same;
  }, { timeout: 10_000 }).toBeGreaterThanOrEqual(3);
}

test('the section links in the bar work, land below the bar and mark the address', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const p = await currentProfile(browser);
  const nav = screen.getByRole('navigation', { name: 'Sections' });
  if (p.viewport.width < 900) {
    // Under 900px the bar has no section links (the page is one short column).
    await expect(nav).toBeHidden();
    await guard.check();
    return;
  }
  await expect(nav).toBeVisible();
  for (const [label, hash] of NAV) {
    await nav.getByRole('link', { name: label }).tap();
    await expect(browser).toHaveURL(new RegExp(`${hash}$`));
    await settled(browser);
    const geo = await browser.evaluate((h: string) => {
      const el = document.querySelector(h) as HTMLElement;
      const heading = (el.querySelector('h2') ?? el) as HTMLElement;
      const bar = (document.querySelector('header.bar') as HTMLElement).getBoundingClientRect();
      const r = heading.getBoundingClientRect();
      return { top: r.top, barBottom: bar.bottom, vh: window.innerHeight, sectionTop: el.getBoundingClientRect().top };
    }, hash);
    expect(geo.top, `${label}: heading hidden under the bar`).toBeGreaterThanOrEqual(geo.barBottom - 1);
    expect(geo.top, `${label}: heading not on screen`).toBeLessThan(geo.vh * 0.8);
  }
  await guard.check();
});

test('the brand link and "Download" in the bar and the hero go where they say', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const p = await currentProfile(browser);
  await browser.evaluate('window.scrollTo(0, 3000)');
  await screen.getByRole('link', { name: 'oneMK, back to the top' }).first().tap();
  await expect(browser).toHaveURL(/#top$/);
  await settled(browser);
  expect(await browser.evaluate<number>('Math.round(window.scrollY)')).toBeLessThan(5);
  // The bar's Download button exists from 480px up and jumps to the download section.
  const barButton = browser.locator('header.bar .bar__end a.btn');
  if (p.viewport.width >= 480) {
    await barButton.tap();
    await expect(browser).toHaveURL(/#download$/);
    await settled(browser);
    const top = await browser.evaluate<number>("Math.round(document.querySelector('#download h2').getBoundingClientRect().top)");
    expect(top).toBeGreaterThan(40);
    expect(top).toBeLessThan(400);
  } else {
    await expect(barButton).toBeHidden();
  }
  await guard.check();
});

test('"Other downloads" under the hero buttons jumps to the download section', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  await screen.getByRole('link', { name: 'Other downloads' }).tap();
  await expect(browser).toHaveURL(/#download$/);
  await settled(browser);
  await expect(screen.getByRole('heading', { name: 'Download oneMK.' })).toBeVisible();
  await guard.check();
});

test('the skip link is first in the tab order, shows itself on focus, and moves focus into the content', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  await browser.keyboard.press('Tab');
  const skip = screen.getByRole('link', { name: 'Skip to content' });
  await expect(skip).toBeFocused();
  const box = await browser.evaluate(() => { const r = (document.activeElement as HTMLElement).getBoundingClientRect(); return { top: r.top, left: r.left, w: r.width, h: r.height, vw: window.innerWidth }; });
  expect(box.top).toBeGreaterThanOrEqual(0);
  expect(box.left).toBeGreaterThanOrEqual(0);
  expect(box.left + box.w).toBeLessThanOrEqual(box.vw);
  await browser.keyboard.press('Enter');
  await expect(browser).toHaveURL(/#main$/);
  // The next Tab goes into the main content, not back up into the bar.
  await browser.keyboard.press('Tab');
  const inMain = await browser.evaluate<boolean>("!!document.activeElement && !!document.activeElement.closest('main')");
  expect(inMain).toBe(true);
  await guard.check();
});

test('the footer links are all present and the footer brand link returns to the top', async ({ app, browser, screen, guard }) => {
  await app.open('/');
  const foot = screen.getByRole('contentinfo');
  for (const name of ['Source on GitHub', 'Releases', 'Documentation', 'Command reference', 'Security model', 'Security policy', 'MIT licence', 'Apache-2.0 licence']) {
    await expect(foot.getByRole('link', { name })).toHaveCount(1);
  }
  await foot.getByRole('link', { name: 'oneMK, back to the top' }).scrollIntoView();
  await foot.getByRole('link', { name: 'oneMK, back to the top' }).tap();
  await expect(browser).toHaveURL(/#top$/);
  await settled(browser);
  expect(await browser.evaluate<number>('Math.round(window.scrollY)')).toBeLessThan(5);
  await guard.check();
});

test('the bar stays on top while scrolling and gains its edge after the first pixels', async ({ app, browser, guard }) => {
  await app.open('/');
  expect(await browser.evaluate<boolean>("document.querySelector('[data-bar]').hasAttribute('data-scrolled')")).toBe(false);
  await browser.evaluate('window.scrollTo(0, 1500)');
  await expect.poll(() => browser.evaluate<boolean>("document.querySelector('[data-bar]').hasAttribute('data-scrolled')")).toBe(true);
  const top = await browser.evaluate<number>("Math.round(document.querySelector('header.bar').getBoundingClientRect().top)");
  expect(top).toBe(0);
  await guard.check();
});
