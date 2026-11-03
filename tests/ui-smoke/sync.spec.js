// Remote sync settings: the folder layout a bundle is written into.
const { test, expect } = require('@playwright/test');
const { openApp, gotoView } = require('./helpers');

const openSync = async (page) => {
  await gotoView(page, '设置');
  await expect(page.locator('.sync-settings')).toBeVisible();
  // Scoped to the one field: `.sync-field input` on its own matches every input
  // in the form, which Playwright rejects as ambiguous.
  return page.locator('.sync-field', { hasText: '远程路径' });
};

test.describe('sync settings', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
  });

  test('the path field names a folder, and says which one', async ({ page }) => {
    const field = await openSync(page);
    // The field used to read "远程文件" and wrote a single JSON blob. Content now
    // lives in a derived folder, so the label and a live hint have to agree.
    await expect(field.locator('input')).not.toHaveValue('');

    const path = await field.locator('input').inputValue();
    const hint = await field.locator('.sync-field-hint').textContent();
    const expected = '/' + path.trim().replace(/\.json$/i, '') + '/';
    expect(hint, `hint should echo the configured path, got ${hint}`).toContain(expected);
  });

  test('the bundle hint cannot push the page sideways', async ({ page }) => {
    // The hint holds a path with `overflow-wrap: anywhere`; a long prefix inside
    // the two-column grid is exactly what used to create scroll.
    const field = await openSync(page);
    for (const width of [430, 390, 320]) {
      await page.setViewportSize({ width, height: 800 });
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - window.innerWidth,
      );
      expect(overflow, `horizontal overflow at ${width}px`).toBeLessThanOrEqual(1);
      const hint = await field.locator('.sync-field-hint').evaluate((el) => ({
        wrap: getComputedStyle(el).overflowWrap,
        width: Math.round(el.getBoundingClientRect().width),
      }));
      expect(hint.wrap).toBe('anywhere');
      expect(hint.width).toBeGreaterThan(0);
    }
  });
});
