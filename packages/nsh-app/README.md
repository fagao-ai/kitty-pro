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

### This app runs no JavaScript, and the binary still contains a JS engine

Worth saying because the two are easy to confuse and the distinction is load-bearing.

The interface is Rust, and it is compiled: `nsh.json` sets `"rust": "."` with `"root": "App"`,
there is no `.tsx` anywhere in the crate, and NSH's own JS host (`nsh-js`, QuickJS) is not in the
dependency tree — `cargo tree` says so and `nm` on the linked binary agrees, zero `nsh_js`
symbols. NSH *does* ship a TSX mode, where UI is TypeScript on QuickJS with hot updates; this
crate simply is not that.

The binary nevertheless carries `rquickjs` symbols (518 in this one, 532 in the Dioxus build),
because the shared backend needs them: `proxy-core`'s config-script override runs the user's
JavaScript through `rquickjs`, so the same engine is in the Dioxus build too. That is a feature
of `proxy-core`, not a sign of a JS front end — it is the same reason the binary carries a Go
archive.

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
| `src/autostart.rs` | The one platform gate a login item needs, in the shape `packages/api` uses |

`nsh_base` supplies behavior and no appearance, so nothing styles a base control at a call
site: a screen composes `controls::`, and a missing variant is added there.

## What is done

All six destinations are ported: Overview, Nodes, Subscriptions, Rules, Logs and Settings.

| Screen | Holds |
| --- | --- |
| 总览 | The system-proxy switch, the exit probe, four figures, the last minute of traffic |
| 节点 | The core's proxy groups and their members, searchable, with latency and one chosen exit |
| 订阅 | Import from a URL or pasted links, refresh one or all, switch the active one, delete |
| 规则 | Custom routing rules and per-domain DNS, on two tabs, with the process picker, plus the QuickJS override on Settings |
| 日志 | The routed connections and the core's own lines, filtered by tally and search |
| 设置 | Appearance and language, where traffic goes, how the machine hands it over, login item, proxy env vars, cloud sync, app updates, the config script, the diagnostic report |

The overview carries the switch that points the operating system at the core. Connecting
opens a listener on 7890; that switch is what makes anything use it, so it stays on 总览 beside
where the connection is made rather than among preferences, and it is parked with a reason when
there is no core running. Its state is read from the OS at boot and re-read after any stop,
since the setting changes underneath the app and a remembered value would overstate what is
connected.

The 登录时启动 switch follows the same rule for the same reason, and reads the operating system
rather than the profile: a login item is a file an installer or a cleanup tool can delete while
the app is closed. On a target with no per-user launch entry the row is absent, not disabled —
`src/autostart.rs` is where that is decided.

Three things are worth knowing about how they behave:

- **The selection and the `proxy` group's entry are one value, written in two places.**
  `api::build_native_config` builds a config whose selector defaults to `selected_tag`, then
  calls `apply_proxy_group_selections`, which overwrites that default with
  `group_selections["proxy"]` — the second field wins. A pick that moved only the first left the
  core dialing the node the user had just replaced, with the list showing the new one and
  nothing on screen to contradict it. `domain::sync_proxy_group_selection` is the only function
  that writes the entry. A *new* choice goes through `store::set_selected`, and every path that
  makes one does: `select_group_outbound` for a press, `activate_nodes` and `after_refresh` when
  a subscription changes. The two places that install a whole map rather than make a choice —
  `apply_profile` on load and the rollback after a refused switch — set both fields from one
  consistent snapshot, which is why they do not go through `set_selected`: a load has to keep the
  other groups' entries exactly as stored. Load-time normalization is also what repairs a file
  that arrived with the two fields disagreeing, the one way they can part without this app's
  fault. `a_stale_group_choice_overrides_the_selection_the_engine_builds` pins the engine's
  ordering, so the fix can be recognised as unnecessary if that ever changes, and
  `a_choice_written_by_another_shell_is_reconciled_on_load` pins the repair.
- **Edits say when they are not live.** Rules, DNS overrides, the config script and switching
  a subscription all reach the engine only through a fresh config, so each one reports
  "重新连接后生效" while the core is running and the overview keeps its reconnect banner up.
  Choosing a node is the exception, and deliberately so: it goes through the live
  `select_proxy_group` call, which needs no restart.
- **Log collection follows the page.** The core streams logs only while asked, so collection
  turns on when the log page opens over a running core and off when either changes. The
  indicator reports the backend's answer rather than the app's wish, so a refusal parks the
  page on 已暂停 instead of claiming a live feed.

### One floating action per page

Each screen carries its own circular button at the bottom-right, and only the page decides what
it does: 总览 starts and stops the proxy, 节点 measures latency, 规则 opens the new-rule form,
日志 pauses and resumes collection, 订阅 opens the import dialog. 设置 has none.

Three things follow from that shape and are worth knowing before changing it:

- **Icon-only, with a required name.** A pill wide enough for 正在申请网络权限 is a banner, not a
  button. The glyph carries the action and its state (`Power`/`Rocket`, `Pause`/`Play`), while
  `accessibility_label` keeps the words for a screen reader — and for the tests, which press by
  `role:button[启动代理]` and have nothing else to hold.
- **The colour is a state, not a decoration.** `domain::DiscSkin` says what an idle and a live
  disc are drawn from: a neutral inset surface with one short shadow, or the accent with its
  glow and halo. An always-accent disc answers nothing, and spending the only loud colour on
  nothing leaves none for what matters. The mapping is in `domain` because a live state needs a
  running core, which no headless test starts — so the rule is unit-tested and only the idle
  half is checked end to end.
- **Pages leave room for it.** A disc over the last row of a list hides what the reader scrolled
  for, which is worse than the bar this replaced. Every page ends in
  `controls::action_clearance()`, derived from the same `ACTION_DISC` the button sizes itself
  from, and `the_floating_action_is_a_disc_at_the_bottom_right` reads that constant rather than
  repeating a number a test would keep passing at.

Both forms on 规则 are hidden until its disc is pressed; they used to sit permanently above
their lists, which made each tab read as a form with some rows under it. The page has one disc,
so its name follows the visible tab — 添加规则 on one, 添加专用 DNS on the other — because a
button that names the hidden tab's form would be lying about what the press does. `编辑` on a
rule row opens that form, filled. Pressing the disc again closes it and discards the draft, and
the glyph changes to say which the next press does.

Notices appear at the top of the window and centred across its whole width, including over the
rail. Placement is asserted through the drawn frames (`a_notice_lands_top_centre`) because a
text check cannot fail on position — the words are identical in any corner.

## The visual language

It is carried over rather than redesigned. `theme.json` defines 45 roles
per appearance and carries every color and radius from `app.css` at its original value,
including the `rgba()` alphas the cards rely on. Three of those roles exist because the CSS has
them and the first port dropped them: `--switch-thumb`, the thumb's opaque fill,
`--switch-thumb-shadow`, the shadow that is the only thing separating a white thumb from an 82%
white track, and `--accent-glow`, the floating action's own shadow colour while it is lit. Where the CSS file also keeps a type scale, font weights, spacing steps and layout
widths as custom properties, NSH has those as typed styles instead, so they are written as
methods (`.text_xs()`, `px(264.0)`) rather than as tokens.

A control whose look is made of parts needs those parts styled at the right level, and this is
where the port's first pass went wrong: `checked_style` paints a `switch`'s own box, not its
track, so the checked fill drew an orange rectangle around a track that had never been sized.
`ui::controls::switch_state` now styles the track and the thumb themselves, takes their sizes
from `domain::SwitchGeometry` (the same numbers `app.css` gives `.switch` and `.switch::after`,
with the thumb's travel derived so it cannot stop matching the inset), and the box paints
nothing at all.

The three variable faces (Space Grotesk, Inter, JetBrains Mono) are the same woff2 bytes the
Dioxus app inlines, extracted from `packages/ui/assets/styling/fonts.css` into
`assets/fonts/`. CJK still falls through to the system font, as it does there.

## What the Dioxus shell has that this one does not

Checked by reading both, not by assumption. What is left:

| Missing | What it means |
| --- | --- |
| Tray, close-to-tray | Menu-bar item and the "closing the window keeps the core running" behaviour. These live in `packages/desktop`, not in `packages/ui`, so they are outside this crate's reach. |
| Web and mobile shells | This crate is native. Under `nsh run --target web` the backend would run in the browser and could not start a core; there is no server mode yet. |

Every screen is ported and every settings card has a home here. The two rows above are the whole
distance between the two front ends, and neither is a decision this crate can make on its own.

One card is distributed rather than copied. The original groups the core's version, its state, the
unavailable-core diagnosis and its retry under one 内核 heading; here those four things live where
each is acted on — the state in the header chip and the about card, the diagnosis and 重新检测内核
on 总览 beside the control they explain, and the override that replaces the config on 设置. A native
window has no scroll budget to spend on a card the user reads once.

### Diagnostics, about and the process picker: how they differ from the original

Both are ported, each in the form this shell can honestly offer.

The about card reports the same figures the Dioxus one does, and `复制诊断信息` produces the same
text. Two things differ. The platform is derived from NSH's own idiom (`Desktop`/`Mobile`) rather
than passed in as a prop by each entry point — there is one binary here, and it changes shape with
the window it is in. And the copy goes through NSH's system clipboard instead of forking `pbcopy`:
the report is meant for a bug report or a chat window, which is what the system clipboard is for.
That difference is visible in the tests. `copy_proxy_environment_variables` shells out, so pressing
it in a test would overwrite whoever runs them; `set_clipboard` is served from the engine's own
clipboard under `Headless`, so `copying_the_report_puts_the_whole_text_on_the_clipboard` presses it
and reads the text back — the only copy action in this crate that is safe to press and worth
asserting on.

The settings search filters by card rather than by row, and its keyword tables live in
`domain::SettingsSection`, so which words a card answers to is a fact the tests can drive without an
engine. A query that matches nothing empties the page, which the page says and undoes in one press.

`选择进程` is the same dialog over the same `list_running_processes`, with the same rule about which
half of a process lands in the field (`domain::process_rule_value`). Its tests cannot name a process
— the list is the machine's — so they assert the contract instead: the picker is offered for the two
process kinds and not for a domain rule, it always reaches an answer rather than sitting on
正在读取, and picking a row fills a field that could not be submitted while empty. On a machine with
no process table the app may enumerate, the dialog says so and the tests take that as the answer.

### Cloud sync, and what its tests can and cannot reach

The card holds the same form as the original: the provider segment, one address field per provider,
the object path with the bundle directory it implies, the credentials, and the four actions
(保存, 下载覆盖本机, 上传, 覆盖远端). `domain::SyncProviderForm::fields` is the single list of
which label edits which stored field, which is what stops the two providers' addresses from ever
being saved into each other's slot. Conflict handling, the plain-language translation of ETag and
baseline refusals, the two-step confirm with its expiry, and the pre-pull backup are all the
backend's, reused as they are.

Two decisions are this crate's own. Nothing on the card is enabled until `sync.json` has actually
been read — the config is held as `Option` rather than defaulted, because saving an untouched
`SyncConfig::default()` over a real one is invisible data loss, and a read that fails parks the
card with a retry instead. And the completeness check that parks the three remote actions runs
before anything is dialled: an address with no path writes to the storage root, a path with no
address cannot be reached at all, and both are knowable without a round trip.

**What the tests cover, and the hole in it.** `sync::push` and `sync::pull` speak HTTP through
`reqwest` directly rather than through NSH's `fetch`, so the suite's `StubNetwork` cannot intercept
them: no test presses 上传, 覆盖远端 or 下载覆盖本机, because doing so would dial the address on
screen. What is covered is every guard that decides whether a press would be allowed, the
provider switch and the fields it swaps, the masked secrets, the arm and its expiry, and 保存同步
设置 end to end — the one press whose whole effect lands in the scratch directory. The remote
round trip itself, the conflict panel's two buttons and the pre-pull backup are reasoned from
`packages/api/src/sync.rs` rather than observed, the same way the live group switch is (see
Unverified). It is worth running once against a real WebDAV host.

### App updates

The card reads the same state file the Dioxus shell writes, from the same three calls, and the
check runs on launch whether or not the settings page is ever opened — the settings read and the
check are separate boot reads because the card must not be the only thing that ever asks. Whether
this build has a channel at all is the backend's answer (`AppUpdateState::Unsupported`) and the
card only reports it: deriving that from the window shape was tried and is wrong, because an NSH
phone build is Android, which is published, and a front-end guess cannot be corrected by the server.

A check that fails is remembered rather than toasted, and is kept apart from the status rather than
folded into it, because "no update" and "could not tell" have to look different — and because the
card is usually not on screen when the launch check fails, so a notice would be gone before anyone
read it. 忽略这个版本 and the daily-check switch are both tested through to `update.json` and back
after a relaunch, since a switch that does not survive a restart is decoration.

The waiting update also reaches the navigation, as it does in the original: once as a notice
(发现新版本 v…, at most one per release — the daily re-check would otherwise repeat it every
launch), and after that as a mark on 设置. `controls::NavMark` is one type covering the rail's two
trailing marks, the update dot and the count chips, because the original keeps them in one slot and
they never appear together. The dot's meaning is carried by the button's *name*
(`设置（有新版本可更新）`), not by its colour: a 7px circle says nothing to a reader, and a label on
a bare span is not reliably announced.

The tests seed a recent `last_check_at` in every scratch directory, which holds the 24-hour throttle
so no launch check reaches the network from a test run. `fetch_release` goes to a hard-coded GitHub
URL through `reqwest`, out of `StubNetwork`'s reach, so 检查更新 is asserted reachable and never
pressed; the check's own round trip belongs to `packages/api`'s tests. What the seeded state *does*
exercise is everything downstream of it — the announcement, the rail mark, the dismiss and the
switch — because those read the same answer the real check would have produced.

### Proxy groups: what was closed, and what was not

The nodes page now lists the groups the core's own config contains and switches a member
through `select_proxy_group`, so a pick moves the running core's `proxy` selector instead of
restarting it — no dropped listener, and with TUN no torn-down routes or DNS.

It is worth being exact about the rest, because the gap this row used to claim was larger than
it is. `build_singbox_config` derives the group set from the node list, and the only group it
produces is one `selector` named `proxy` holding every node. A config script cannot add a
second: `ALLOWED_SCRIPT_KEYS` does not admit `outbounds`, and `reject_clash_config` refuses a
script that returns `proxy-groups`. The Clash YAML parser never reads a provider's own
`proxy-groups` section at all — `parser.rs` has no concept of a group — so it keeps the nodes
and drops the grouping. A provider that ships nodes grouped by region is therefore a flat list
here, but it is a flat list in the Dioxus shell too, and in the core: that is a decision in
`proxy-core`'s parser, not a gap in this front end, and no screen in this crate can offer a
group the engine does not build. Closing it for real means teaching the parser to carry
provider groups into the config, which is a change to the shared backend.

`nsh.json` carries `icon` (a copy of `assets/icon/kitty-pro-1024.png`, which is what
`nsh package` wants: a square PNG), version and id, so a packaged build is named, versioned
and icons correctly without flags.

## Tests

`cargo test -p nsh-app-kitty` runs the screens under NSH's real engine. Three things are
worth knowing about them:

- `Screen::texts` reports only what a user can see, so a text in a view that measures
  nothing is a failed assertion. Several layout bugs appeared this way.
- `Screen` answers questions about *content* — text, selectors, a11y state — and nothing about
  appearance. That blind spot is how every switch in the app was visibly broken while the suite
  stayed green. `Headless::observe` fixes it: `tests/app.rs` records the `MountItem` stream the
  engine hands a real platform (each view's frame and its `ViewStyle`) and rebuilds the tree
  from it, so a test can assert what would actually be drawn.
  `a_switch_draws_a_track_and_a_thumb_that_moves` reads the track's and thumb's sizes, the
  thumb's fill, alpha and shadow, and the checked slide from there — and each of those was
  checked by putting the bug back and watching the test fail.
- Each test takes an exclusive scratch `KITTY_PROFILE_DIR`, because the screens read and
  write the real profile store. `tests/fixtures/profile-two-nodes.json` is generated from
  `AppProfile` itself, so a pass means the port reads what the Dioxus app writes.

`every_status_pair_clears_aa_contrast` reads `theme.json` and checks every status color
against its own background in both appearances, so a token added later is covered by the
same rule that catches the existing ones.

The suite deliberately never presses 启动代理. That call starts the real sing-box core, which
hangs a test run and leaves a live proxy behind; the connect path is covered up to the guard
(`connect_is_off_until_there_is_something_to_connect_to`, `a_refused_press_starts_nothing`)
and no further. The same rule keeps 测试连通 unpressed (over a running core it is a real
outbound request), and 登录时启动 and 复制代理环境变量 unpressed too: the first writes a launch
entry into whoever's account runs the tests, the second overwrites their clipboard. Each is
asserted as far as it can be without the side effect — present, named, settled, and unreachable
in the state where a press would cost something. 复制诊断信息 is the exception, and the reason is
in the section above: NSH's clipboard belongs to the test, so that one can be pressed and read
back.

A dialog needs `Headless::answer_dialogs_myself` before it is pressed. Left at its default the
driver answers a dialog on the app's behalf and picks cancel, so a test that means to confirm
would silently see the cancelled branch.

Two selector traps, each of which here produced a test that passed for the wrong reason:

- `app.shows(x)` compares `x` against a **whole text node**, so
  `!app.shows("selector")` cannot fail when the word only ever appears inside a longer
  sentence. Phrase assertions go through `shows_matching`, which wraps `Screen::matches` and so
  takes a selector, regex form included. Every negative assertion written that way is paired
  with a positive one, since a selector matching nothing at all satisfies a negative just as
  happily as the real thing does.
- Waiting for `内核已就绪` proves the core answered, not that the profile did — they are two
  independent boot reads. A control whose state comes from the profile has to wait for the
  profile's own evidence, which is why `connect_is_off_until_there_is_something_to_connect_to`
  waits for the seeded node's name. Getting this wrong is a failure that only shows up on a
  loaded machine.

## Unverified

No screenshot, and this time that was checked rather than assumed. `screencapture` on this
machine answers `could not create image from display`, and NSH's headless `screenshot()` refuses
outright: "a screenshot needs a real platform, which draws the app". What the tests prove is
every text measured, non-zero, inside the window and reachable by the tab ring, plus — since the
switch fix — the frames, fills, alpha and transforms a platform is actually told to draw for the
one control read that way. That is still not a looked-at result: rhythm, alignment and whether
something *looks* right remain unseen, and only one control is covered by the stream assertions.
Before trusting the visual result, run
`cargo run -p nsh-app-kitty --features desktop` and check both appearances and a phone-width
window. The user's screenshot of a broken toggle is what started this: the suite was green at the
time, so "green" means "no test looked at it".

The live half of the group switch has the same problem in miniature. `select_proxy_group`
against a running core is the path that avoids a restart, and the suite starts no core, so what
is covered is the saved-and-reported branch, the guards, and the persistence; the
switch-without-restart behaviour is reasoned from `api::select_proxy_group` and
`singbox::select_outbound` rather than observed. It is worth watching once by hand.
