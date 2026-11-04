// The update card, and the wasm clock trap it used to walk into.
//
// A `wasm32` client has no platform clock: `SystemTime::now()` compiles and then
// traps at run time, which reads in the browser as `RuntimeError: unreachable`
// and kills the whole app, not just the card. Nothing in `cargo test` can see
// that, because the native test target *does* have a clock. These guards mount
// the settings page - the one view that renders the card - and insist the app is
// still alive afterwards.
//
// The endpoints are stubbed (see `stubUpdateCheck`). That has to happen before
// `openApp`, because the check runs from the app's own startup effect rather
// than when the card mounts: a route installed afterwards would only catch the
// button, and the card would render whatever the real GitHub answer happened to
// be. Stubbing also keeps the suite off GitHub's anonymous request budget and
// lets every card state be chosen here instead of inherited from the network.
const { test, expect } = require('@playwright/test');
const { gotoView, openApp, stopStubs, stubUpdateCheck } = require('./helpers');

// Any trap in the wasm module surfaces as an uncaught error on the page.
const watchForPanic = (page) => {
  const panics = [];
  page.on('pageerror', (error) => panics.push(String(error)));
  return panics;
};

/// Boot the app against one chosen check answer and return the card.
///
/// Only the settings read is awaited here. Each test then asserts the badge with
/// a retrying `toHaveText`, which is what waits out the check itself: a
/// non-retrying comparison would read the placeholder this returns and lose the
/// race against an answer that is a few milliseconds away.
const openUpdates = async (page, options) => {
  const fixture = await stubUpdateCheck(page, options);
  await openApp(page);
  await gotoView(page, '设置');
  const card = page.locator('.update-settings');
  await expect(card).toBeVisible();
  const badge = card.locator('.status-badge').first();
  await expect
    .poll(async () => (await badge.innerText()).trim(), { timeout: 30000 })
    .not.toBe('读取中');
  return { badge, card, fixture };
};

test.describe('update card', () => {
  test.afterEach(async ({ page }) => {
    await stopStubs(page);
  });

  test('mounting the settings page does not trap the wasm client', async ({ page }) => {
    const panics = watchForPanic(page);
    const { card } = await openUpdates(page, { variant: 'available' });
    // The ages are the clock-reading part, so the card must render them before
    // the app is declared alive.
    await expect(card.getByText('当前版本')).toBeVisible();
    // Re-render a second time with whatever the check returned: a trap on the
    // first paint and a trap after the answer arrives are different bugs.
    await gotoView(page, '概览');
    await gotoView(page, '设置');
    await expect(page.locator('.update-settings')).toBeVisible();
    expect(panics, `wasm trapped: ${panics.join(' | ')}`).toEqual([]);
  });

  test('an available release names one asset and offers the release page', async ({ page }) => {
    const panics = watchForPanic(page);
    const { badge, card, fixture } = await openUpdates(page, { variant: 'available' });
    await expect(badge).toHaveText('有更新');

    const release = card.locator('.update-release');
    await expect(release).toBeVisible();
    const asset = release.locator('.update-asset');
    await expect(asset).toBeVisible();
    // Assert on the name rather than the row, so the size cannot satisfy a
    // pattern meant for a file name.
    const name = (await asset.locator('span').first().innerText()).trim();
    expect(name).toBe(fixture.latest.asset);
    // A download decision is made in MiB, so a bare byte count is a bug.
    await expect(asset.locator('small')).toHaveText(/MiB$/);

    const link = card.getByRole('link');
    await expect(link).toHaveAttribute('href', fixture.latest.url);
    // An age is rendered, which is the card reading the server's clock because a
    // wasm client has no clock of its own.
    await expect(release.locator('small').first()).toContainText('天前');
    expect(panics, `wasm trapped: ${panics.join(' | ')}`).toEqual([]);
  });

  // The release page link stays for this case on purpose - the page still lists
  // the other platforms - so what is pinned is that the card names the missing
  // package rather than showing an asset it does not have.
  test('a release with no package for this machine says which one is missing', async ({
    page,
  }) => {
    const { badge, card } = await openUpdates(page, { variant: 'no_asset' });
    await expect(badge).toHaveText('有更新');
    await expect(card.locator('.update-asset')).toContainText('此发布未包含适配本机的安装包');
    // Exactly one asset row, and it is the explanation rather than a file name.
    await expect(card.locator('.update-asset')).toHaveCount(1);
    await expect(card.locator('.sync-security-note')).toContainText('已按');
  });

  test('being current says so and offers no release to read', async ({ page }) => {
    const { badge, card } = await openUpdates(page, { variant: 'up_to_date' });
    await expect(badge).toHaveText('已是最新');
    // No release block, so nothing to download and nothing to ignore: the check
    // button is the only action the card may offer here.
    await expect(card.locator('.update-release')).toHaveCount(0);
    await expect(card.getByRole('link')).toHaveCount(0);
    await expect(card.getByRole('button', { name: '忽略此版本' })).toHaveCount(0);
  });

  test('a failed check leaves a readable status, not a placeholder', async ({ page }) => {
    const { badge, card } = await openUpdates(page, { variant: 'failed' });
    await expect(badge).toHaveText('检查失败');
    await expect(card.locator('[role="alert"]')).toContainText('dns failure');
  });

  test('a transport failure is reported by the card, not swallowed', async ({ page }) => {
    // The branch with no answer object at all. It must not render as "尚未检查" -
    // the same thing the badge said before the check ran - because a background
    // check deliberately does not raise a toast for this.
    const { badge } = await openUpdates(page, { transportError: true });
    await expect(badge).toHaveText('检查失败');
  });

  // The dot is the only part of the feature a user sees without opening
  // settings, and it is the only signal that is colour alone.
  test('an available release marks the settings item, a current build does not', async ({
    page,
  }) => {
    const { card } = await openUpdates(page, { variant: 'available' });
    await expect(card).toBeVisible();
    // The sidebar copy only. At this viewport the phone nav is `display: none`
    // but still in the document, and a locator matching both would trip
    // Playwright's strict mode rather than test anything.
    const settings = page.locator('.primary-nav .nav-item').filter({ hasText: '设置' });
    await expect(settings.locator('.nav-dot')).toBeVisible();
    // Assistive tech reads the dot's meaning off the item's name, because a 7px
    // circle carries colour and nothing else.
    await expect(settings).toHaveAttribute('aria-label', /有新版本可更新/);

    await stopStubs(page);
    await openUpdates(page, { variant: 'up_to_date' });
    await expect(settings.locator('.nav-dot')).toHaveCount(0);
    // The accessible name comes and goes with the dot, so a build that is
    // current never announces an update it does not have.
    await expect(settings).not.toHaveAttribute('aria-label', /有新版本可更新/);
  });

  test('the card cannot push the page sideways on a phone', async ({ page }) => {
    const { card } = await openUpdates(page, { variant: 'available' });
    for (const width of [430, 390, 360, 320]) {
      await page.setViewportSize({ width, height: 800 });
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - window.innerWidth,
      );
      expect(overflow, `horizontal overflow at ${width}px`).toBeLessThanOrEqual(1);
      await expect(card).toBeVisible();
    }
  });
});
