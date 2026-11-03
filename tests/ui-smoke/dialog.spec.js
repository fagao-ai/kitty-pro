// Dialog behaviour: Escape dismisses, Tab cannot leave the panel, focus returns
// to the trigger, and a panel is always a viewport overlay.
const { test, expect } = require('@playwright/test');
const { gotoView, openApp, settlePanel } = require('./helpers');

const openImport = async (page) => {
  await gotoView(page, '概览');
  await page.getByRole('button', { name: '添加订阅' }).click();
  return page.locator('#import-dialog');
};

const focusInfo = (page) =>
  page.evaluate(() => {
    const el = document.activeElement;
    const dialog = el && el.closest ? el.closest('[role="dialog"]') : null;
    return {
      tag: el ? el.tagName : null,
      dialogId: dialog ? dialog.id : null,
      label: el ? (el.getAttribute('aria-label') || el.getAttribute('title') || el.textContent || '').trim().slice(0, 20) : null,
    };
  });

test.describe('dialogs', () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
  });

  test('exactly one shell renders every dialog', async ({ page }) => {
    const panel = await openImport(page);
    await expect(panel).toBeVisible();
    await expect(panel).toHaveAttribute('role', 'dialog');
    await expect(panel).toHaveAttribute('aria-modal', 'true');
    await expect(panel).toHaveAttribute('aria-label', '添加订阅');
    // The backdrop belongs to the shell, so no view may paint its own.
    await expect(page.locator('.modal-backdrop')).toHaveCount(1);
  });

  test('escape dismisses and focus lands back on the trigger', async ({ page }) => {
    const trigger = page.getByRole('button', { name: '添加订阅' });
    await trigger.click();
    await expect(page.locator('#import-dialog')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('[role="dialog"]')).toHaveCount(0);
    // Without focus restore the keyboard lands on document.body.
    await expect.poll(() => trigger.evaluate((el) => el === document.activeElement)).toBe(true);
  });

  test('tab cannot escape the panel', async ({ page }) => {
    await openImport(page);
    for (let i = 0; i < 14; i += 1) {
      await page.keyboard.press('Tab');
      await expect
        .poll(() => page.evaluate(() => {
          const dialog = document.activeElement && document.activeElement.closest
            ? document.activeElement.closest('[role="dialog"]')
            : null;
          return dialog ? dialog.id : null;
        }), { message: `focus left the dialog on tab ${i}` })
        .toBe('import-dialog');
    }
  });

  test('shift-tab wraps to the last control', async ({ page }) => {
    await openImport(page);
    // The trap walks the *reachable* controls: the submit button starts disabled
    // (the source field is empty), so the ring skips it exactly as the browser
    // skips it when tabbing.
    const prepared = await page.evaluate(() => {
      const dialog = document.getElementById('import-dialog');
      const list = () =>
        Array.from(
          dialog.querySelectorAll(
            'button:not([disabled]), input:not([disabled]), textarea:not([disabled])',
          ),
        );
      list()[0].focus();
      return list().length;
    });
    expect(prepared).toBeGreaterThan(1);

    await page.keyboard.press('Shift+Tab');
    const wrapped = await page.evaluate(() => {
      const dialog = document.getElementById('import-dialog');
      const list = Array.from(
        dialog.querySelectorAll(
          'button:not([disabled]), input:not([disabled]), textarea:not([disabled])',
        ),
      );
      return {
        index: list.indexOf(document.activeElement),
        last: list.length - 1,
        inDialog: dialog.contains(document.activeElement),
      };
    });
    expect(wrapped.inDialog, 'focus stayed inside the panel').toBe(true);
    expect(wrapped.index, 'wrapped to the last reachable control').toBe(wrapped.last);
  });

  test('the panel stays a viewport overlay', async ({ page }) => {
    await openImport(page);
    const box = await settlePanel(page, '#import-dialog');
    const size = page.viewportSize();
    // Centred by the backdrop, which is fixed and full-viewport.
    expect(Math.abs(box.x + box.width / 2 - size.width / 2)).toBeLessThan(4);
    expect(Math.abs(box.y + box.height / 2 - size.height / 2)).toBeLessThan(4);
    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(1);
  });

  test('script editor keeps its own width', async ({ page }) => {
    // `.script-modal` and `.modal` tie on specificity, so file order decides:
    // the editor used to render at the 510px import-dialog width.
    await gotoView(page, '设置');
    await page.getByRole('button', { name: '编辑 JavaScript 配置覆写' }).click();
    const panel = page.locator('#script-editor-dialog');
    await expect(panel).toBeVisible();
    const width = (await panel.boundingBox()).width;
    expect(width).toBeGreaterThan(800);
  });

  test('nested dialog returns focus to the outer trigger', async ({ page }) => {
    // The process picker opens on top of the inline rule editor. It used to be
    // painted inside the rule card, whose `backdrop-filter` makes that card the
    // containing block for `position: fixed` - the picker then centred on the
    // card and, once the rule list scrolled, left the viewport entirely.
    await gotoView(page, '规则');
    const add = page.getByRole('button', { name: '添加规则' });
    if (!(await add.count())) test.skip(true, 'rule editor needs a writable profile');
    await add.click();
    const editor = page.locator('.rule-editor');
    await expect(editor).toBeVisible();

    const matchTrigger = editor.locator('.rule-select-trigger').nth(1);
    await matchTrigger.click();
    await page.getByRole('option', { name: '进程名' }).click();

    const pick = page.getByRole('button', { name: '选择进程' });
    await expect(pick).toBeVisible();
    await pick.click();

    const picker = page.locator('#process-picker-dialog');
    await expect(picker).toBeVisible();
    const box = await settlePanel(page, '#process-picker-dialog');
    const size = page.viewportSize();
    expect(box, 'picker must be on screen').not.toBeNull();
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.y + box.height).toBeLessThanOrEqual(size.height + 1);
    expect(Math.abs(box.x + box.width / 2 - size.width / 2)).toBeLessThan(4);
    // The picker is a sibling of the card, not a descendant of it.
    expect(await picker.evaluate((el) => !!el.closest('.rules-workspace'))).toBe(false);
    await expect
      .poll(() => focusInfo(page).then((info) => info.dialogId))
      .toBe('process-picker-dialog');

    // Escape closes only the topmost layer.
    await page.keyboard.press('Escape');
    await expect(picker).toHaveCount(0);
    await expect(editor).toBeVisible();
    await expect
      .poll(() => focusInfo(page).then((info) => info.label))
      .toContain('选择进程');
  });

  for (const height of [470, 430]) {
    test(`dialogs stay usable at ${height}px tall`, async ({ page }) => {
      await page.setViewportSize({ width: 1440, height });
      await openImport(page);
      const box = await settlePanel(page, '#import-dialog');
      expect(box.y).toBeGreaterThanOrEqual(0);
      expect(box.y + box.height).toBeLessThanOrEqual(height + 1);
      // The panel scrolls internally, so its actions are never stranded.
      await expect(page.locator('#import-dialog .modal-actions button').last()).toBeVisible();
    });
  }
});
