// Negative control: re-create the bugs this pass fixed and assert the suite's
// own budgets reject them. A layout guard that can never fail is worse than no
// guard, so this file is the evidence the numbers in layout.spec.js bite.
//
// Opt-in, because it only passes by *injecting* broken CSS:
//   KITTY_NEGCHECK=1 npx playwright test negcheck.spec.js
const { test, expect } = require('@playwright/test');
const {
  LIMITS,
  openApp,
  gotoView,
  measureScroller,
  stubCoreLogStream,
} = require('./helpers');

// The regressions, restored: a viewport-height floor on the cards, a fixed
// height console that clips its rows, and static column captions.
const BUGS = `
  .main-content { display: block !important; }
  .workspace-section { min-height: calc(100vh - 126px) !important; }
  .log-workspace {
    height: calc(100vh - 132px) !important;
    flex: none !important;
    overflow: hidden !important;
  }
  .log-workspace .log-list { overflow: visible !important; flex: none !important; }
  .log-workspace .log-table-header { position: static !important; }
`;

test('negative control: bugs are detectable', async ({ page }) => {
  test.skip(!process.env.KITTY_NEGCHECK, 'opt-in: run with KITTY_NEGCHECK=1');

  await stubCoreLogStream(page);
  await page.goto('/');
  await page.waitForSelector('.primary-nav button');
  await page.addStyleTag({ content: BUGS });

  // Bug 2: the `calc(100vh - 126px)` floor made four card pages scroll 27px
  // into a strip of background at every window height.
  await gotoView(page, '规则');
  const rules = await measureScroller(page);
  expect(rules.phantom, 'bug 2: the card must not scroll past the fold').toBeGreaterThan(
    LIMITS.phantom,
  );

  // Bug 1: the console was a fixed-height box with `overflow: hidden` and no
  // scroll container, so rows past the fold were unreachable by wheel or touch.
  await gotoView(page, '日志');
  await expect(page.locator('.log-row')).not.toHaveCount(0, { timeout: 20000 });
  const clipped = await page.evaluate(() => {
    const card = document.querySelector('.log-workspace');
    const list = document.querySelector('.log-list');
    const last = list.querySelector('.log-row:last-child');
    return {
      overflowY: getComputedStyle(list).overflowY,
      cardBottom: Math.round(card.getBoundingClientRect().bottom),
      lastBottom: Math.round(last.getBoundingClientRect().bottom),
    };
  });
  expect(clipped.overflowY, 'bug 1: the row list must be the scroller').not.toBe('auto');
  expect(
    clipped.lastBottom - clipped.cardBottom,
    'bug 1: rows must not fall below the clipping card',
  ).toBeGreaterThan(LIMITS.clip);

  const logs = await measureScroller(page);
  expect(
    logs.dead,
    `bug 1: content past the scroll height is unreachable (dead=${logs.dead})`,
  ).toBeLessThan(-LIMITS.clip);

  // Bug 5 follow-up: sticky captions are what keeps context while rows scroll.
  const sticky = await page
    .locator('.log-workspace .log-table-header')
    .evaluate((el) => getComputedStyle(el).position);
  expect(sticky, 'bug 5: column captions should have gone sticky').not.toBe('sticky');
});
