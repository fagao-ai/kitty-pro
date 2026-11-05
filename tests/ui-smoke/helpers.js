// Shared helpers for the layout smoke suite.

const { expect } = require('@playwright/test');

const VIEWS = ['概览', '节点', '订阅', '规则', '日志', '设置'];

// Widths spanning every breakpoint in app.css (1600 / 1280 / 980 / 760 / 720 /
// 420 / 359), the boundary neighbours 721 and 320, and a few widths in between.
// A regression in a media query shows up as horizontal overflow, which no view
// may ever produce.
const WIDTHS = [
  1620, 1600, 1440, 1280, 1180, 1024, 980, 860, 760, 721, 720, 620, 430, 390,
  360, 359, 320,
];

const navLabel = (view) => view;

/// Load the app and wait until it is actually interactive.
///
/// Dioxus fullstack server-renders the first frame, so the markup exists before
/// any handler is attached: the shell is clickable but inert until the wasm
/// finishes downloading and hydrates. The first `/api/` round-trip is the
/// evidence that polling started, so the listener is installed before
/// navigation to avoid racing it.
async function openApp(page) {
  let booted = false;
  page.on('response', (response) => {
    if (response.url().includes('/api/')) booted = true;
  });
  await page.goto('/');
  await page.waitForSelector('.primary-nav button', { timeout: 60000 });
  await expect
    .poll(() => booted, { message: 'app never hydrated (no API traffic)', timeout: 60000 })
    .toBe(true);
}

async function gotoView(page, view) {
  const clicked = await page.evaluate((label) => {
    const items = Array.from(
      document.querySelectorAll('.primary-nav button, .mobile-nav button'),
    );
    const hit = items.find((el) => (el.textContent || '').includes(label));
    if (!hit) return false;
    hit.click();
    return true;
  }, navLabel(view));
  if (!clicked) throw new Error(`navigation item missing for view ${view}`);
  await page.waitForFunction(
    (label) => {
      const h1 = document.querySelector('.topbar h1');
      return h1 && h1.textContent.trim() === label;
    },
    navLabel(view),
    { timeout: 15000 },
  );
}

/// Geometry of the page scroller plus the deepest piece of content inside it.
/// `dead` is the space the scroller offers past the last visible element: the
/// phantom scroll a `calc(100vh - N)` floor leaves behind. `negative` means
/// content is clipped past the scroll height, i.e. unreachable.
async function measureScroller(page) {
  return page.evaluate(() => {
    const mobile = window.matchMedia('(max-width: 760px)').matches;
    const scroller = mobile ? document.querySelector('.main-content') : document.documentElement;
    const rect = scroller.getBoundingClientRect();
    const base = mobile ? rect.top : 0;
    const padding = scroller === document.documentElement
      ? parseFloat(getComputedStyle(document.querySelector('.main-content')).paddingBottom) || 0
      : parseFloat(getComputedStyle(scroller).paddingBottom) || 0;

    let deepest = 0;
    for (const el of document.querySelectorAll('.main-content *')) {
      const cls = typeof el.className === 'string' ? el.className : '';
      // Dioxus dev-overlay toasts are fixed-position noise, not app content.
      if (el.id.startsWith('dx') || cls.includes('dx-toast') || cls.includes('toast-viewport')) continue;
      const style = getComputedStyle(el);
      if (style.display === 'none' || style.visibility === 'hidden') continue;
      if (parseFloat(style.opacity) < 0.05) continue;
      const box = el.getBoundingClientRect();
      if (box.width < 2 || box.height < 2) continue;
      const bottom = box.bottom - base + scroller.scrollTop;
      if (bottom > deepest) deepest = bottom;
    }

    const metrics = {
      mobile,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
      horizontal: scroller.scrollWidth - window.innerWidth,
      scrollHeight: scroller.scrollHeight,
      clientHeight: scroller.clientHeight,
      deepest: Math.round(deepest),
      // Space the scroller offers past the last element: negative means content
      // is clipped and unreachable.
      dead: Math.round(scroller.scrollHeight - padding - deepest),
      // Scrollable distance.
      phantom: Math.max(0, scroller.scrollHeight - scroller.clientHeight),
    };
    // Scroll that reveals nothing. The reader may only scroll as far as the last
    // element actually requires, so anything past that distance is a `100vh`
    // magic number in disguise. `dead` is the slack after the last element and
    // `phantom` the scrollable distance, so the wasted overlap is their minimum:
    //   content fits  -> phantom 0, excess 0    (a short page never scrolls)
    //   content long  -> dead ~0,   excess ~0    (scroll ends at the last row)
    //   buggy 100vh   -> phantom 27, dead 302, excess 27  (a strip of background)
    metrics.excessScroll = Math.min(metrics.phantom, Math.max(0, metrics.dead));
    return metrics;
  });
}

/// Internal overflow of `.main-content` alone.
///
/// `measureScroller()` compares the page scroller's `scrollWidth` with the
/// window, but `body { overflow-x: hidden }` can hide a child that is wider than
/// its column: the document stays put while the content itself overflows or is
/// clipped. Comparing `scrollWidth` to `clientWidth` on the container catches
/// that case. Kept as a separate export so `measureScroller`'s signature (and
/// every existing caller) stays untouched.
async function measureMainContent(page) {
  return page.evaluate(() => {
    const el = document.querySelector('.main-content');
    if (!el) throw new Error('.main-content is missing; cannot measure internal overflow');
    return el.scrollWidth - el.clientWidth;
  });
}

const routeEntry = (sequence, decision) => ({
  sequence,
  // Unique per row so a test can tell the newest entry apart after a batch.
  timestamp: `12:${String(Math.floor(sequence / 60) % 60).padStart(2, '0')}:${String(sequence % 60).padStart(2, '0')}`,
  level: 'info',
  message: `outbound/hy2[HY-1] connection to example-${sequence}.com:443`,
  route: {
    decision,
    target: `example-${sequence}.com:443`,
    host: `example-${sequence}.com`,
    port: 443,
    target_kind: 'domain',
    outbound_type: 'hy2',
    outbound_tag: 'HY-1',
    outbound_chain: ['节点→代理→出口'],
    source_ip: '192.168.1.24',
  },
});

const DECISIONS = ['proxy', 'direct', 'proxy', 'block'];

/// Drive the log console from a fake kernel instead of a real core: the poll
/// loop only runs while the status endpoint reports `running`, and its batches
/// are what the follow-latest chip counts. `firstBatch` rows arrive at once so
/// the console has something to scroll; `pollBatch` rows arrive every second.
async function stubCoreLogStream(page, { firstBatch = 80, pollBatch = 3 } = {}) {
  let cursor = 0;

  await page.route('**/api/core/status', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ state: 'running', version: 'kitty-pro-embedded/1.13.14', note: null }),
    });
  });

  await page.route('**/api/core/logs/collection', (route) =>
    route.fulfill({ status: 200, contentType: 'application/json', body: 'null' }),
  );

  await page.route('**/api/core/logs', async (route) => {
    const size = cursor === 0 ? firstBatch : pollBatch;
    const entries = [];
    for (let index = 0; index < size; index += 1) {
      cursor += 1;
      entries.push(routeEntry(cursor, DECISIONS[cursor % DECISIONS.length]));
    }
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ next_cursor: cursor, entries }),
    });
  });

  return () => cursor;
}

async function stopStubs(page) {
  await page.unrouteAll({ behavior: 'ignoreErrors' });
}

/// A release document in the exact shape `check_app_update` answers with.
///
/// Field names and the snake_case state strings are the serialized shape of
/// `api::update::AppUpdateStatus`/`AppRelease`, so this has to move with those
/// types - which is a feature: a renamed field breaks the card here rather than
/// silently in front of a user.
const releaseStatus = (overrides = {}) => ({
  state: 'available',
  current_version: '1.0.9',
  latest: {
    version: '1.1.0',
    title: 'v1.1.0',
    url: 'https://github.example/releases/tag/v1.1.0',
    notes: '- fixed something\n\n**Full Changelog**: https://github.example/compare',
    published_at: '2026-11-01T00:00:00Z',
    prerelease: false,
    asset: 'KittyPro_1.1.0_aarch64.dmg',
    asset_size: 23351810,
  },
  last_check_at: 1_793_704_246,
  snoozed: false,
  target: '桌面版 · macos · aarch64',
  error: null,
  // Ages are measured against this, so it sits a whole three days past
  // `published_at` for the labels to read sanely.
  server_time: 1_793_704_246 + 3 * 86_400,
  ...overrides,
});

const baseRelease = releaseStatus().latest;

const UPDATE_VARIANTS = {
  available: releaseStatus(),
  up_to_date: releaseStatus({ state: 'up_to_date', latest: null }),
  // Newer release, nothing published for this OS/CPU pair.
  no_asset: releaseStatus({ latest: { ...baseRelease, asset: null, asset_size: 0 } }),
  // A check that reached the server and failed there: an answer with a reason.
  failed: releaseStatus({
    state: 'failed',
    latest: null,
    error: '检查更新失败: dns failure',
    server_time: 1_793_704_246,
  }),
};

/// Answer the update endpoints from the browser instead of GitHub.
///
/// Two reasons this is not optional for the update card:
///
/// 1. The real endpoint contacts `api.github.com`, whose anonymous limit is 60
///    requests per hour per IP. CI runners share egress IPs, so a suite that
///    reaches the network several times per run is a flaky suite.
/// 2. Whatever the newest published release happens to be decides what renders,
///    so an unstubbed suite can only ever assert the one case the repository is
///    currently in. Stubbing lets each card state - available, current, no asset
///    for this machine, failed - be pinned on purpose.
///
/// `variant` picks one of `UPDATE_VARIANTS`; `transportError` answers 500
/// instead, which is the branch with no `AppUpdateStatus` at all.
async function stubUpdateCheck(page, { variant = 'available', transportError = false } = {}) {
  const document = UPDATE_VARIANTS[variant];
  if (!document) throw new Error(`unknown update fixture ${variant}`);

  await page.route('**/api/update/settings', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ auto_check: true, last_check_at: 1_793_704_246 }),
    }),
  );

  await page.route('**/api/update/check', (route) =>
    route.fulfill(
      transportError
        ? {
            status: 500,
            contentType: 'application/json',
            // The shape `ServerFnError::into_response` actually produces. The
            // client falls back to "HTTP 500: <text>" when it cannot read this
            // as its own payload, so what matters is the status: either way the
            // call resolves to `Err` and no `AppUpdateStatus` exists at all.
            body: JSON.stringify({ error: '读取更新状态失败' }),
          }
        : { status: 200, contentType: 'application/json', body: JSON.stringify(document) },
    ),
  );

  await page.route('**/api/update/dismiss', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ auto_check: true, last_check_at: 1_793_704_246 }),
    }),
  );

  return document;
}

/// Wait until a freshly opened panel stops moving, and return its box.
///
/// `.modal` runs a 170ms entry animation (translateY + scale), so a box read
/// straight after the click is mid-flight and several pixels low. Polling for a
/// stable box is safer than sleeping: it holds whether or not the animation is
/// collapsed by `prefers-reduced-motion`.
async function settlePanel(page, selector) {
  const panel = page.locator(selector);
  let previous = null;
  for (let attempt = 0; attempt < 30; attempt += 1) {
    const box = await panel.boundingBox();
    if (
      previous &&
      Math.abs(previous.y - box.y) < 0.5 &&
      Math.abs(previous.height - box.height) < 0.5
    ) {
      return box;
    }
    previous = box;
    await page.waitForTimeout(40);
  }
  return panel.boundingBox();
}

// Layout budgets. Shared with the negative control so that test cannot drift
// away from what the real assertions allow.
const LIMITS = {
  // Horizontal: nothing may ever push the document wider than the window.
  horizontal: 1,
  // Pages whose card is sized to the fold must not scroll at all.
  phantom: 2,
  // Scroll past the last element: the reader may only travel as far as content.
  excessScroll: 2,
  // Content must never end up past the scroll height (that is unreachable).
  clip: 4,
};

module.exports = {
  LIMITS,
  VIEWS,
  WIDTHS,
  gotoView,
  openApp,
  settlePanel,
  measureScroller,
  measureMainContent,
  stubCoreLogStream,
  stubUpdateCheck,
  UPDATE_VARIANTS,
  stopStubs,
};
