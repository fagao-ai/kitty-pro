# UI layout smoke test

Headless Chromium guards for the bugs that `cargo test` cannot see: what the
layout engine actually does when the window changes size, and whether a dialog
still behaves like a dialog.

## Running it

Serve a **release** web build and point the suite at it.

```sh
dx build --package web --web --fullstack true --release --debug-symbols false --locked

stage=target/dx/web/release/web
mkdir -p /tmp/kitty-web
cp "$stage/server" /tmp/kitty-web/kitty-pro-web-server
cp -R "$stage/public" /tmp/kitty-web/public
cd /tmp/kitty-web && PORT=7897 ./kitty-pro-web-server --daemon --log-file ./web.log

cd tests/ui-smoke
npm ci
npx playwright install chromium     # or KITTY_BROWSER=chrome to reuse a local Chrome
npm test
```

`KITTY_WEB_URL` selects the target (default `http://127.0.0.1:7897`).

Never point this at `dx serve` unless you have waited for the 80 MB debug wasm to
hydrate: the server-rendered shell looks ready but has no event handlers, so
every click silently does nothing and the dialog tests hang. The suite waits for
the first `/api/` round-trip before touching the page, which is the evidence that
the app booted.

## What each file covers

- `layout.spec.js` - 320-1620px wide with no horizontal overflow; the card pages
  (概览/节点/订阅/规则/日志) must not scroll at all on a normal window, because
  any scroll there is the old `calc(100vh - 126px)` floor; short windows and
  phones may scroll, but only onto real content; the sidebar footer stays inside
  a 380px-tall window; shrinking and growing back leaves no stale layout.
- `console.spec.js` - log rows are reachable (the console scrolls internally
  rather than clipping with `overflow: hidden`), the newest row stays on screen
  while following, scrolling away parks the reader and counts what arrives, the
  chip jumps back to the top, and the sticky captions stay opaque. Runs against
  a stubbed core (`/api/core/status` reports `running`, `/api/core/logs` hands
  out batches once a second), so it needs no proxy and no subscription.
- `dialog.spec.js` - one shell renders every dialog; Escape dismisses and returns
  focus to the trigger; Tab cannot leave the panel; Shift+Tab wraps to the last
  *reachable* control; panels stay centred in the viewport; the script editor
  keeps its 880px; the nested process picker lands on screen instead of inside
  the rule card's containing block.
- `negcheck.spec.js` - opt-in proof that the budgets above can fail (see below).

## The negative control

`npm run test:negcheck` injects the original broken CSS at runtime and asserts
the suite rejects it. Run it whenever you loosen a limit: a layout guard that
cannot fail is worse than no guard.

```sh
KITTY_NEGCHECK=1 npx playwright test negcheck.spec.js
```

It is skipped in a normal run because it only passes against deliberately broken
styles.

