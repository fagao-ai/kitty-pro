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
the app booted. A debug wasm is therefore unusable for UI tests: it is exactly
why the suite needs the release build and stays out of the PR quality gate
(`ci.yml`), living in `ui-smoke.yml` instead (see below).

## What each file covers

- `layout.spec.js` - the real CSS breakpoints (1600 / 1280 / 980 / 760 / 720 /
  420 / 359, plus boundary neighbours 721 and 320) with no horizontal overflow,
  checked both on the document and inside `.main-content` (`measureMainContent()`:
  `body overflow-x:hidden` otherwise hides an overflowing column); the 720px
  breakpoint keeps the DNS row / DNS editor inputs inside the column; the card
  pages (概览/节点/订阅/规则/日志) must not scroll at all on a normal window,
  because any scroll there is the old `calc(100vh - 126px)` floor; short windows
  and phones may scroll, but only onto real content; the sidebar footer stays
  inside a 380px-tall window; shrinking and growing back leaves no stale layout.
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
- `update.spec.js` - mounting the settings page cannot trap the wasm client. A
  `wasm32` build has no platform clock (`SystemTime::now()` compiles, then traps as
  `RuntimeError: unreachable` and takes the whole app down), so the ages the update
  card prints have to come from the machine that ran the check. This is the only
  guard that can see that class of bug: the native test target does have a clock.
  The update endpoints are stubbed (`stubUpdateCheck`), which matters twice over:
  the live check spends GitHub's 60-requests-per-hour anonymous limit from a shared
  CI egress IP, and unstubbed the suite can only assert whatever release the
  repository happens to have published. Stubbing pins each card state on purpose -
  an available release naming one asset, a release with no package for this
  machine, a current build offering nothing to download, a check that failed, and
  a transport failure that produced no answer at all (the badge must not sit on
  "尚未检查", since a background check does not raise a toast). Also pins the
  settings nav dot and its accessible name, and that the card cannot create
  horizontal overflow on a phone. The fixtures are the serialized shape of
  `api::update::AppUpdateStatus`, so renaming a field breaks them deliberately.
- `sync.spec.js` - the remote-path field names a folder and its hint echoes the
  configured path's stem (not a hard-coded name), and that hint cannot create
  horizontal overflow on a phone.
- `a11y.spec.js` - native-Playwright accessibility guards plus an automated
  axe-core scan (`@axe-core/playwright`, wcag2a/wcag2aa/best-practice, both
  themes, every view) across all six views,
  in both light and dark themes: every `button` has an accessible name (a failure
  names the index and selector), every `.segmented-control button` carries
  `aria-pressed`, a Tab press leaves a visible focus ring, and 320px / 1600px
  produce no horizontal overflow (document **and** `.main-content` internal
  overflow). No `@axe-core/playwright` and no new npm dependency.
- `flow.spec.js` (F23) - lightweight, network-free flow checks: the import
  dialog's collapsible「支持的格式」help, the disabled submit plus the
  empty-address hint, Escape closing with no leftover / reopened dialog; the
  overview empty state's「添加订阅」CTA; and that the「重新连接」banner is absent
  by default. All `/api/*` calls are stubbed (`isolateNetwork` reuses
  `stubUpdateCheck`), so no real subscription, core, or network is involved.
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

## CI 门禁

- `.github/workflows/ci.yml` (P0) - 全量 Rust 门禁，触发于 `pull_request`、
  `push` 到 `main` 与 `workflow_dispatch`。单个 `quality` job 跑在
  `ubuntu-latest`，依次执行 `cargo fmt --all -- --check`、
  `cargo clippy --workspace --all-targets -- -D warnings`、
  `cargo test --workspace`、`cargo test -p desktop --features desktop`、
  `cargo test -p singbox --features embedded-core`（另有一个
  `continue-on-error: true` 的 `cargo audit`，不让第三方 advisory 阻塞 PR）。
  它按 README「Go toolchain」章节显式安装 Go 1.24 —— `cargo test --workspace`
  会编译 sing-box 的 Go bridge，而 sing-box 1.13.14 要求 Go 1.24。在此之前
  `release.yml` 只有 Windows job 跑 `go test ./...` + `cargo test --package
  singbox`，`ui-smoke.yml` 只跑 Playwright，fmt / clippy / workspace 测试完全
  没有门禁覆盖。
- Playwright UI smoke **不进 PR 门禁**：它依赖 release wasm 产物（debug wasm
  ~84 MB 在 CI 里永远无法完成 hydrate），要先 `dx build --release` 再起 web
  server，成本高。它继续由 `.github/workflows/ui-smoke.yml` 负责（UI 路径的
  PR、`main` 推送与 `workflow_dispatch`）。该 workflow 跑 `npm test`，spec
  列表维护在 `tests/ui-smoke/package.json`；新增 spec 必须同步加进 `test`
  脚本，否则 CI 会静默跳过它（negcheck 单独用 `test:negcheck` 触发）。

