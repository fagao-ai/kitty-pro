# Kitty Pro on Native Shell

The NSH rewrite of this app's interface. It is a second front end, added alongside the
Dioxus one rather than replacing it: `packages/ui`, `web`, `desktop` and `mobile` still
build, still pass, and are untouched.

```sh
cargo run -p nsh-app-kitty --features desktop   # a real window, no NSH CLI needed
cargo test -p nsh-app-kitty                     # the screens under NSH's engine, headless
nsh run                                         # every target, with hot updates (needs the nsh CLI)
```

`desktop` is a Cargo feature because the Linux host needs GTK 4.14 development packages,
which the CI runners do not carry. The default build pulls no system libraries, so
`cargo check --workspace` stays green on Ubuntu.

## The backend is shared, not rewritten

`api`, `proxy-core` and `singbox` are used as they are. An NSH app is a native process, so
the server-function hop the Dioxus shells make is simply a function call here, and the
`#[kitty_server_fn]` macro adds no HTTP attribute without the `server` feature. Nothing in
`src/` re-implements profile storage, the sing-box control plane, rule parsing or sync.

## Layout of the crate

| Path | Holds |
| --- | --- |
| `nsh.json` | NSH's manifest: the crate to build, the root component, the theme file |
| `theme.json` | Every color, radius and font. Nothing else in the crate writes a color literal |
| `src/lib.rs` | The native module, the root component, theme and font loading |
| `src/main.rs` | The desktop runner behind `--features desktop` |
| `src/store.rs` | All app state, and the only code that changes it |
| `src/shell.rs` | The rail, the header, the phone tab bar, and screen routing |
| `src/ui/` | The styled layer: `theme.rs` token accessors, `controls.rs`, `icons.rs` |
| `src/pages/` | One module per screen |
| `src/domain.rs` | Copy and units, free of the engine so `cargo test` can reach them |

`nsh_base` supplies behavior and no appearance, so nothing styles a base control at a call
site: a screen composes `controls::`, and a missing variant is added there.

## What is done

All six destinations are ported: Overview, Nodes, Subscriptions, Rules, Logs and Settings.

| Screen | Holds |
| --- | --- |
| 总览 | The connect control, four figures, the last minute of traffic |
| 节点 | The active subscription's nodes, searchable, with latency and one chosen exit |
| 订阅 | Import from a URL or pasted links, refresh one or all, switch the active one, delete |
| 规则 | Custom routing rules and per-domain DNS, on two tabs, plus the QuickJS override on Settings |
| 日志 | The routed connections and the core's own lines, filtered by tally and search |
| 设置 | Appearance, where traffic goes, how the machine hands it over, the config script |

The overview carries the switch that points the operating system at the core. Connecting
opens a listener on 7890; that switch is what makes anything use it, so it sits beside the
connect control rather than among preferences, and it is parked with a reason when there is
no core running. Its state is read from the OS at boot and re-read after any stop, since the
setting changes underneath the app and a remembered value would overstate what is connected.

Two things are worth knowing about how they behave:

- **Edits say when they are not live.** Rules, DNS overrides, the config script and switching
  a subscription all reach the engine only through a fresh config, so each one reports
  "重新连接后生效" while the core is running and the overview keeps its reconnect banner up.
- **Log collection follows the page.** The core streams logs only while asked, so collection
  turns on when the log page opens over a running core and off when either changes. The
  indicator reports the backend's answer rather than the app's wish, so a refusal parks the
  page on 已暂停 instead of claiming a live feed.

## The visual language

It is carried over rather than redesigned. `theme.json` defines 42 roles
per appearance and carries every color and radius from `app.css` at its original value,
including the `rgba()` alphas the cards rely on. Where the CSS file also keeps a type scale,
font weights, spacing steps and layout widths as custom properties, NSH has those as typed
styles instead, so they are written as methods (`.text_xs()`, `px(264.0)`) rather than as
tokens.

The three variable faces (Space Grotesk, Inter, JetBrains Mono) are the same woff2 bytes the
Dioxus app inlines, extracted from `packages/ui/assets/styling/fonts.css` into
`assets/fonts/`. CJK still falls through to the system font, as it does there.

## What the Dioxus shell has that this one does not

Checked by reading both, not by assumption. These are the features of `packages/ui` with no
counterpart here, roughly in the order a user would miss them:

| Missing | What it means |
| --- | --- |
| Proxy groups | The original lists the core's `selector`/`urltest` groups and lets a member be chosen (`preview_proxy_groups`, `select_proxy_group`). Here a subscription's nodes are a flat list and only one node is chosen. A provider whose config groups its nodes by region is not selectable in this UI. |
| Cloud sync | Push, pull, conflict resolution and the pre-pull backup (`sync_push`, `sync_pull`, `commit_sync_pull`, plus the sync config form). |
| App updates | Version check, the "有新版本可更新" affordance, dismiss, and the update settings. |
| Start at login | `api::autostart`. |
| Connectivity probe | The overview's 测试连通, which dials through the running core and reports the result. |
| Diagnostics and about | Core version, node/subscription/rule/DNS counts, the language row, and the settings search box. |
| Copy proxy env vars | `copy_proxy_environment_variables`, which the desktop tray offers. |
| Process picker | The rule editor offers 选择进程 from `list_running_processes`; here a process name is typed. |
| Tray, close-to-tray | Menu-bar item and the "closing the window keeps the core running" behaviour. These live in `packages/desktop`, not in `packages/ui`, so they are outside this crate's reach. |
| Web and mobile shells | This crate is native. Under `nsh run --target web` the backend would run in the browser and could not start a core; there is no server mode yet. |

`nsh.json` carries `icon` (a copy of `assets/icon/kitty-pro-1024.png`, which is what
`nsh package` wants: a square PNG), version and id, so a packaged build is named, versioned
and icons correctly without flags.

## Tests

`cargo test -p nsh-app-kitty` runs the screens under NSH's real engine. Two things are
worth knowing about them:

- `Screen::texts` reports only what a user can see, so a text in a view that measures
  nothing is a failed assertion. Every layout bug found so far appeared this way.
- Each test takes an exclusive scratch `KITTY_PROFILE_DIR`, because the screens read and
  write the real profile store. `tests/fixtures/profile-two-nodes.json` is generated from
  `AppProfile` itself, so a pass means the port reads what the Dioxus app writes.

`every_status_pair_clears_aa_contrast` reads `theme.json` and checks every status color
against its own background in both appearances, so a token added later is covered by the
same rule that catches the existing ones.

The suite deliberately never presses 启动代理. That call starts the real sing-box core, which
hangs a test run and leaves a live proxy behind; the connect path is covered up to the guard
(`connect_is_off_until_there_is_something_to_connect_to`, `a_refused_press_starts_nothing`)
and no further.

A dialog needs `Headless::answer_dialogs_myself` before it is pressed. Left at its default the
driver answers a dialog on the app's behalf and picks cancel, so a test that means to confirm
would silently see the cancelled branch.

## Unverified

No screenshot. Headless tests prove structure, bindings and behavior; they cannot prove
that a part which draws nothing was styled, or that the spacing reads as intended. This
environment has no screen-recording access and the NSH CLI is a 620 MB release asset that
downloads too slowly to fetch, so nothing here has been looked at. Before trusting the
visual result, run `cargo run -p nsh-app-kitty --features desktop` and check both
appearances and a phone-width window.
