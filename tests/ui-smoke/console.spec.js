// Log console: the rows have to stay reachable, and new rows must not move the
// reader who scrolled back through them.
const { test, expect } = require('@playwright/test');
const { gotoView, openApp, stubCoreLogStream, stopStubs } = require('./helpers');

const listState = (page) =>
  page.evaluate(() => {
    const list = document.querySelector('.log-list');
    if (!list) return null;
    const rows = Array.from(list.querySelectorAll('.log-row'));
    const last = rows[rows.length - 1];
    const box = list.getBoundingClientRect();
    return {
      scrollTop: Math.round(list.scrollTop),
      scrollHeight: list.scrollHeight,
      clientHeight: list.clientHeight,
      overflowY: getComputedStyle(list).overflowY,
      rows: rows.length,
      firstTime: rows[0] ? rows[0].querySelector('.log-time').textContent : null,
      // Is the oldest row inside the visible part of the console? If the card
      // clips with `overflow: hidden` and no scroller this is unreachable.
      lastRowVisible: last
        ? last.getBoundingClientRect().bottom <= box.bottom + 1 &&
          last.getBoundingClientRect().top >= box.top - 1
        : false,
      // The row the reader's eye is anchored to: the first row still poking into
      // the visible box, plus how far its top sits from the list's top edge. A
      // parked reader is "not moved" iff BOTH stay constant while rows arrive —
      // scrollTop itself is expected to grow, because that growth is exactly how
      // the browser's scroll anchoring holds this row still (see below).
      anchorTime: (() => {
        const anchor = rows.find((r) => r.getBoundingClientRect().bottom > box.top + 1);
        return anchor ? anchor.querySelector('.log-time').textContent : null;
      })(),
      anchorOff: (() => {
        const anchor = rows.find((r) => r.getBoundingClientRect().bottom > box.top + 1);
        return anchor ? Math.round(anchor.getBoundingClientRect().top - box.top) : null;
      })(),
      chip: (document.querySelector('.log-follow-button') || {}).textContent || null,
    };
  });

test.describe('log console', () => {
  test.beforeEach(async ({ page }) => {
    await stubCoreLogStream(page);
    await openApp(page);
    await gotoView(page, '日志');
    await expect(page.locator('.log-row')).not.toHaveCount(0, { timeout: 20000 });
  });

  test.afterEach(async ({ page }) => {
    await stopStubs(page);
  });

  test('every row is reachable, not clipped by the card', async ({ page }) => {
    const state = await listState(page);
    expect(state.overflowY).toBe('auto');
    expect(state.scrollHeight).toBeGreaterThan(state.clientHeight);

    // Wheel to the bottom and confirm the last row lands inside the console.
    const box = await page.locator('.log-list').boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    for (let i = 0; i < 40; i += 1) {
      await page.mouse.wheel(0, 400);
      if ((await listState(page)).lastRowVisible) break;
    }
    expect((await listState(page)).lastRowVisible, 'oldest row must be scrollable into view').toBe(true);
  });

  test('follows the newest row while parked at the top', async ({ page }) => {
    const before = await listState(page);
    expect(before.scrollTop).toBe(0);
    expect(before.chip).toBeNull();

    // Poll batches arrive once per second; the newest row has to stay on screen.
    await page.waitForTimeout(3500);
    const after = await listState(page);
    expect(after.scrollTop).toBe(0);
    expect(after.rows).toBeGreaterThan(before.rows);
    expect(after.firstTime).not.toBe(before.firstTime);
    expect(after.chip, 'no chip while following').toBeNull();
  });

  test('scrolling away parks the reader and counts what arrives', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('.log-list').scrollTop = 400;
    });
    // The chip only appears once a batch lands after the reader parked, so wait
    // for it instead of guessing how long the poll takes.
    await expect(page.locator('.log-follow-button')).toBeVisible({ timeout: 10000 });
    const away = await listState(page);
    expect(away.scrollTop).toBeGreaterThan(0);
    expect(away.chip).toMatch(/条新日志/);

    await page.waitForTimeout(3000);
    const grown = await listState(page);
    expect(grown.rows, 'rows keep arriving').toBeGreaterThan(away.rows);
    // "The viewport must not move" means the row the reader is looking at stays
    // put, NOT that scrollTop stays constant. Rows arrive above the parked
    // viewport, and the browser's scroll anchoring compensates by growing
    // scrollTop by exactly the height inserted above — that growth is what
    // holds the anchor row visually still. Asserting a frozen scrollTop number
    // would contradict the anchoring this feature relies on (it passed only
    // because the original blind-written spec never ran against a real DOM).
    expect(grown.anchorTime, 'the parked row stays the anchor row').toBe(away.anchorTime);
    expect(Math.abs((grown.anchorOff ?? 0) - (away.anchorOff ?? 0)), 'the parked row must not drift').toBeLessThanOrEqual(1);
    expect(grown.scrollTop, 'anchoring must compensate by pushing scrollTop down').toBeGreaterThan(away.scrollTop);

    const count = (chip) => Number(chip.match(/(\d+)/)[1]);
    // Rows only ever arrive above the parked viewport, so every extra row in the
    // buffer is one more the reader missed.
    expect(count(grown.chip) - count(away.chip)).toBe(grown.rows - away.rows);
  });

  test('the chip jumps back to the newest row', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('.log-list').scrollTop = 500;
    });
    await expect(page.locator('.log-follow-button')).toBeVisible({ timeout: 5000 });
    await page.locator('.log-follow-button').click();
    await expect
      .poll(async () => (await listState(page)).scrollTop, { timeout: 5000 })
      .toBe(0);
    expect((await listState(page)).chip).toBeNull();

    // Following resumes: the next batch must not raise the chip again.
    await page.waitForTimeout(2500);
    expect((await listState(page)).chip).toBeNull();
  });

  test('sticky captions stay opaque over scrolled rows', async ({ page }) => {
    await page.evaluate(() => {
      document.querySelector('.log-list').scrollTop = 300;
    });
    const style = await page.locator('.log-workspace .log-table-header').evaluate((el) => {
      const computed = getComputedStyle(el);
      return {
        position: computed.position,
        background: computed.backgroundColor,
        image: computed.backgroundImage,
      };
    });
    expect(style.position).toBe('sticky');
    // A translucent-only background lets the row text underneath bleed through.
    expect(style.background, 'sticky header needs an opaque base').not.toBe('rgba(0, 0, 0, 0)');
    expect(style.image).not.toBe('none');
  });
});
