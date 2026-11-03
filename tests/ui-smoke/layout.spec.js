// Resize regressions.
//
// Every bug found by hand in this pass showed up as either horizontal overflow
// or vertical "dead" space (the scroller offering room past the last element,
// or clipping past it). Both are measurable, so they are asserted here across a
// viewport matrix instead of by eye.
const { test, expect } = require('@playwright/test');
const { LIMITS, VIEWS, WIDTHS, gotoView, openApp, measureScroller } = require('./helpers');

// The four pages whose card is sized to fill the fold (plus the overview) must
// not scroll at all on a normal desktop window: they hold a handful of rows, so
// any scrollable distance is the `calc(100vh - 126px)` signature - a card taller
// than its content by exactly whatever the magic number got wrong (27px at
// 1440x900, 183px on the logs page).
const FOLD_VIEWS = ['概览', '节点', '订阅', '规则', '日志'];
// Settings is a genuinely long page and has to scroll.
const FOLD_HEIGHTS = [1000, 900, 768, 700];
// Below this the console's 340px floor legitimately pushes the card past the
// fold, so the page scrolls on purpose; only wasted scroll is a bug there.
const SHORT_HEIGHTS = [620, 500, 420];
const SCROLL_LIMIT = LIMITS.phantom;
const CLIP_LIMIT = LIMITS.clip;

test.describe('viewport resize', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
  });

  for (const width of WIDTHS) {
    test(`no view overflows horizontally at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 800 });
      for (const view of VIEWS) {
        await gotoView(page, view);
        const metrics = await measureScroller(page);
        expect(
          metrics.horizontal,
          `horizontal overflow in ${view} at ${width}px`,
        ).toBeLessThanOrEqual(LIMITS.horizontal);
      }
    });
  }

  for (const height of FOLD_HEIGHTS) {
    test(`card pages fill the fold without scrolling at 1440x${height}`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height });
      for (const view of FOLD_VIEWS) {
        await gotoView(page, view);
        const metrics = await measureScroller(page);
        expect(
          metrics.phantom,
          `${view} scrolls ${metrics.phantom}px at 1440x${height} with no content below the fold`,
        ).toBeLessThanOrEqual(SCROLL_LIMIT);
        expect(metrics.dead, `${view} clipped at 1440x${height}`).toBeGreaterThanOrEqual(-CLIP_LIMIT);
      }
    });
  }

  for (const height of SHORT_HEIGHTS) {
    test(`short windows waste no scroll at 1440x${height}`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height });
      for (const view of VIEWS) {
        await gotoView(page, view);
        const metrics = await measureScroller(page);
        // Scrolling is allowed here - the console keeps a floor so the empty
        // state is never squashed - but it must end at real content.
        expect(metrics.excessScroll, `dead scroll in ${view} at 1440x${height}`).toBeLessThanOrEqual(
          SCROLL_LIMIT,
        );
        expect(metrics.dead, `clipped content in ${view} at 1440x${height}`).toBeGreaterThanOrEqual(
          -CLIP_LIMIT,
        );
      }
    });
  }

  test('no wasted scroll on phones, where the scroller is .main-content', async ({ page }) => {
    for (const width of [430, 390, 360, 320]) {
      for (const height of [880, 760, 568, 420]) {
        await page.setViewportSize({ width, height });
        for (const view of VIEWS) {
          await gotoView(page, view);
          const metrics = await measureScroller(page);
          expect(metrics.mobile, `expected the phone scroller at ${width}px`).toBe(true);
          expect(
            metrics.excessScroll,
            `wasted scroll in ${view} at ${width}x${height}`,
          ).toBeLessThanOrEqual(SCROLL_LIMIT);
          expect(
            metrics.dead,
            `clipped content in ${view} at ${width}x${height}`,
          ).toBeGreaterThanOrEqual(-CLIP_LIMIT);
        }
      }
    }
  });

  test('short windows keep the sidebar status readable', async ({ page }) => {
    // The sidebar is fixed and used to grow past the viewport, pushing the
    // "core ready + build" footer off screen with no way to scroll to it.
    await page.setViewportSize({ width: 1200, height: 380 });
    await gotoView(page, '概览');
    const footer = page.locator('.sidebar-footer');
    await expect(footer).toBeVisible();
    const box = await footer.boundingBox();
    const nav = await page.locator('.primary-nav').evaluate((el) => ({
      scrollHeight: el.scrollHeight,
      clientHeight: el.clientHeight,
      overflowY: getComputedStyle(el).overflowY,
    }));

    expect(box.y + box.height, 'sidebar footer must sit inside the viewport').toBeLessThanOrEqual(381);
    expect(nav.overflowY, 'nav is the region allowed to scroll').toBe('auto');
    expect(nav.scrollHeight).toBeGreaterThan(nav.clientHeight);
  });

  test('resizing back up does not leave stale layout', async ({ page }) => {
    // Shrink and grow without a reload, which is how a window actually gets
    // dragged. Anything that caches a viewport height would break here.
    const steps = [1440, 1181, 1179, 981, 979, 761, 759, 431, 429, 761, 981, 1181, 1440];
    for (const width of steps) {
      await page.setViewportSize({ width, height: 820 });
      await page.waitForTimeout(120);
    }
    await gotoView(page, '规则');
    const metrics = await measureScroller(page);
    expect(metrics.horizontal).toBeLessThanOrEqual(1);
    expect(metrics.phantom).toBeLessThanOrEqual(SCROLL_LIMIT);
    expect(metrics.viewportWidth).toBe(1440);
  });
});
