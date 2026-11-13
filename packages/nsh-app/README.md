# Kitty Pro on Native Shell

This is the app's interface. The Dioxus front end that was here first — `packages/ui`,
`web`, `desktop`, `mobile` — has been deleted, so everything below describes the only
shipped UI rather than a port in progress.

```sh
cargo run -p kitty-pro --features desktop   # a real window, no NSH CLI needed
cargo test -p kitty-pro                     # the screens under NSH's engine, headless
nsh run                                         # every target, with hot updates (needs the nsh CLI)
nsh package --target macos                      # a distributable .app
```

`desktop` is a Cargo feature because the Linux host needs GTK 4.14 development packages,
which the CI runners do not carry. The default build pulls no system libraries, so
`cargo check --workspace` stays green on Ubuntu.

## The backend is shared, not rewritten

`api`, `proxy-core` and `singbox` are used as they are. An NSH app is a native process, so
there is no transport hop at all: every backend call is a plain async function call into
`api`. That is also why `api` no longer depends on Dioxus — with one native shell left, its
server functions and their `ServerFnError` lost their reason to exist, and it now returns
its own `ApiError`. Nothing in `src/` re-implements profile storage, the sing-box control
plane, rule parsing or sync.

### This app runs no JavaScript, and the binary still contains a JS engine

Worth saying because the two are easy to confuse and the distinction is load-bearing.

The interface is Rust, and it is compiled: `nsh.json` sets `"rust": "."` with `"root": "App"`,
there is no `.tsx` anywhere in the crate, and NSH's own JS host (`nsh-js`, QuickJS) is not in the
dependency tree — `cargo tree` says so and `nm` on the linked binary agrees, zero `nsh_js`
symbols. NSH *does* ship a TSX mode, where UI is TypeScript on QuickJS with hot updates; this
crate simply is not that.

The binary nevertheless carries `rquickjs` symbols, because the shared backend needs them:
`proxy-core`'s config-script override runs the user's JavaScript through `rquickjs`, exactly as
it did for the shell this crate replaced. That is a feature of `proxy-core`, not a sign of a JS
front end — it is the same reason the binary carries a Go
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
| `src/domain.rs` | Copy and units, and the platform's rules — each one a free function taking the platform as an argument, so a desktop test asserts a phone's answer |
| `src/autostart.rs` | The one platform gate a login item needs, in the shape `packages/api` uses |

`nsh_base` supplies behavior and no appearance, so nothing styles a base control at a call
site: a screen composes `controls::`, and a missing variant is added there.

## What is done

All six destinations are ported: Overview, Nodes, Subscriptions, Rules, Logs and Settings.

| Screen | Holds |
| --- | --- |
| 总览 | The system-proxy switch (desktops only), the exit probe, four figures, the last minute of traffic |
| 节点 | The core's proxy groups and their members, searchable, with latency and one chosen exit |
| 订阅 | Import from a URL or pasted links, refresh one or all, switch the active one, delete |
| 规则 | Custom routing rules and per-domain DNS, on two tabs, with the process picker, plus the QuickJS override on Settings |
| 日志 | The routed connections and the core's own lines, filtered by tally and search |
| 设置 | Appearance and language, where traffic goes, how the machine hands it over, login item, cloud sync, app updates, the config script, the diagnostic report — the last of those cards' rows belong to a desktop, so the cards are withheld elsewhere (see [The same screens on a phone](#the-same-screens-on-a-phone)) |

The overview carries the switch that points the operating system at the core. Connecting
opens a listener on 7890; that switch is what makes anything use it, so it stays on 总览 beside
where the connection is made rather than among preferences, and it is parked with a reason when
there is no core running. Its state is read from the OS at boot and re-read after any stop,
since the setting changes underneath the app and a remembered value would overstate what is
connected.

**Phones do not draw it at all** — `SystemProxy::is_offered`. A phone has no per-app proxy
setting for the app to point at the port: its routes belong to whichever `VpnService` is up,
and that is this app's own tunnel, so the row could only ever be drawn parked beside a line
explaining that it does nothing. Withholding it follows `process_rules_supported`, which hides
a match kind for the same reason. The decision is *not* a `cfg!` on the wasm target: the web
channel's page is wasm, yet the proxy it changes belongs to the machine running the server, so
everywhere that is not a phone the row follows the backend's `supported` answer, so a browser
driving a Linux box keeps the control it needs. A desktop does not wait for that read either:
`Loading` draws, the row arrives with the card, and only a platform that answers "no such
setting" takes it back out.

The probe row is the one row in the card whose label is a sentence, and the row is a
`justify_between` with a button: without `flex_1` *and* `min_w(px(0.0))` on the label — the
pair the switch row above already used — the hint keeps its intrinsic width and the button is
laid off the right edge of a phone. `min_w(px(0.0))` is the part that carries it, because a
flex item's floor is its content size, so `flex_1` alone cannot shrink a line below the text
itself. `the_probe_row_fits_a_phone` measures both halves on the drawn frames at 390pt: the
old layout still drew every word, which is what let it ship — the suite had been asserting the
words.

The stale-config banner beside it had the same row and was still broken, which says the guard is
easy to leave out when a page assembles the row itself. It goes through `controls::inline_note_with`
now — the control the sync card's failure and the subscription card's hint already use, and the one
place that owns "a sentence and the action that recovers from it". Its English sentence asks for
436pt where its Chinese asks for 126pt, so the row measured 重新连接 at x 584 of a 390pt screen:
the one action that applies a pending change, off screen, in the language where the sentence needs
the most room. `a_note_with_its_action_fits_a_phone` measures the control's frames at 390pt, in
English, through the sync card — the only note of that shape the suite can reach, since the banner
needs a running core and the suite starts none. Taking the guard out of the control fails it with
the action at x 1199 of 390.

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
- **The traffic poll survives a core that has not answered yet.** It reads once a second and
  keeps 运行时长 off the clock rather than one tick per successful read, and only a disconnect or
  a newer generation ends it — `domain::traffic_poll_tick` owns that decision. Ending on a failed
  read is what froze a phone's 总览 at `00:00` and a flat chart beside a card saying 运行中:
  `VpnService` reports the tunnel up before the core behind it answers, and on Android that
  refusal comes back as the bridge's error *text*, which fails to parse as JSON, so the poll died
  on its own first tick. Silence is not absence — which is the opposite of the log poll above, and
  correctly so: that page parks its own indicator and says 已暂停, so silence there is explained.
  The poll also holds a slot for the generation it is watching, the way the log poll holds
  `log_poll`, because `refresh_status` starts one on every read that answers running without
  bumping the generation. Two loops over one connection read twice a second, which halves every
  rate the chart takes from a one-second difference, and each reset 运行时长 to zero on the way
  in — so a press of 重新检测内核 on a healthy core used to zero the figure and leave a second
  writer behind.

### 简体中文 and English, chosen on the settings page

The user asked for both languages ("把 中英文语言页支持了吧, 现在只有中文"). The picker is a segment
of two on the appearance card, and each option is named **in the language it selects** — 简体中文
and English, always — because the reader who switched into a language by accident has to find the
way out without reading the words that would describe it. `AppProfile::language` stores the code,
read and written beside `dark_mode`.

Four things about the design are load-bearing:

- **The Chinese string is the key** (`tr("总览")`), not an id. The app was written in Chinese and
  its screens are the product a Chinese reader expects; `nav.overview` would put a second naming
  system over the code and leave every call site holding two strings, one of which nobody reads.
  `i18n::ENGLISH` is a `const` array rather than a `match` so the collision test can walk it.
- **Every label reads through a closure**: `text(move || tr("总览"))`, never `text("总览")`. Nothing
  can rebuild an NSH app from inside (`remount` belongs to the host), so a language has to
  propagate as a reactive read like the theme does. The second form translates correctly at
  startup and then sits there in Chinese forever — and `the_chrome_is_fully_english_when_english_is_on`
  caught exactly that mistake twice while this was being written, in a label and a heading built
  outside their closures.
- **The stored choice outranks the system locale, and an absent choice is Chinese — not the
  system's.** NSH's own fallback locale is `en-US`, so keying off the platform would have flipped
  every existing install to English on the update that introduces the switch, for readers who never
  chose anything.
- **One test sweeps the whole app, and it is not enough.**
  `english_leaves_no_chinese_on_any_screen` switches to English, walks all six pages by their new
  names and fails on any Chinese left on any of them — reporting the page and the string. It also
  asserts it saw over 100 strings, because "nothing I collected is Chinese" passes trivially on an
  empty collection, and `Screen::texts` reports only laid-out text: a page that rendered nothing
  would otherwise satisfy it perfectly. The one allowed string is `简体中文`, a language option's own
  name, which stays in the language it selects on purpose. It passed while 59 strings were still
  untranslated, which is what the next two bullets are about.
- **What a sweep cannot see, a source scan reads.** A folded form is closed, a toast lasts four
  seconds, a conflict panel needs a second device and a rule kind needs a core that can report
  processes. `every_drawn_string_has_an_english_row` reads the crate's own sources instead and
  requires every Chinese string literal in production code to appear in `ENGLISH`, so the table's
  completeness is a property of what is *written* rather than of what a test happened to render.
  Three exceptions are listed by name and reason: the settings search's keyword soup (searched, not
  drawn, so it must keep the Chinese words even on an English page), the `contains` queries in
  `classify_sync_error` (they match the *backend's* sentence, which is Chinese whatever the
  interface says), and the diagnostic report (a support artifact). It also asserts it read over 400
  Chinese literals, since a scan that found no sources would report no gaps.
- **A row in the table is only half of it: something has to ask for it in English.** The second
  class of miss is a string that has a row and never gets looked up — a `domain` label dropped into
  a raw `text()`, a `SelectItem` title, an icon control's accessibility name, a placeholder. The
  rule pickers were the clearest case: a closed `choice_field` draws its own title through a
  lookup, so a form reading `Exact domain` said nothing about the five items behind the trigger.
  `english_holds_in_the_surfaces_a_page_sweep_cannot_open` opens each hidden surface *by its English
  name* — the press itself fails if the name stayed Chinese — and waits for a string that only the
  open popup contains, so a popup that refused to open cannot report itself clean.
- **A page rebuilds when the language changes, so most labels need no closure.** `screen_body`'s
  `dynamic` closure reads `i18n::lang()` beside the current screen, and a `dynamic` region re-runs
  its closure when a signal read *inside it* changes — which builds the page again in the new
  language. That is what lets ~450 call sites keep writing `text("节点")`. Labels in the rail and
  header, which live outside that region, read `tr` inside their own bindings instead.
- **Composed sentences are translated as templates.** `format!("缓冲区 {n} 条记录")` produces a
  string no table can hold, so `i18n::fill` looks the *template* up with its holes in place and
  substitutes afterwards — the only way English can put the count where English puts it. A unit
  test checks each template's English has the same number of holes as its Chinese, because a
  missing one shows a literal `{}` on screen.
- **`domain` stays pure.** The four time-ago formatters (`relative_age`, `age_label`,
  `since_label`, `auto_check_label`) take a `Lang` argument and use `i18n::translate` — the half of
  the module that needs no reactive runtime — so their unit tests still run on a bare thread and
  state which language they expect. The views pass `store.language()`, which is also what makes
  those notes re-render on a switch.
- **An untranslated string falls back to its Chinese** rather than vanishing, which is the right
  failure while the table is being filled. `i18n`'s own fallback test pins it.

### A window with no title bar

The user asked for a borderless window: the native bar across the top was, in their words, 很丑.
`set_title_bar(TitleBarStyle::Hidden, Some(px(44.0)))` is called once from the root's boot effect.

It is a hidden title bar rather than a frameless window, on purpose: the traffic lights stay, and
a window whose close button is gone can only be left from the Dock or `Cmd+Q`. What hiding it
changes is that the app's own drawing now reaches the top edge and the lights float over it, so
two things follow:

- The rail reserves `TITLE_BAR_HEIGHT` (44pt) above its brand, because the lights sit exactly
  there. `the_rail_leaves_room_for_the_window_buttons` measures the rail's highest text and fails
  if it climbs into that band — verified by deleting the band, which put 品牌 at 2pt, under the
  close button. 44 is the tallest height AppKit still treats as compact chrome.
- The rail's band and the page header both set `windowDrag`, since with no title bar they are the
  only thing to grab. A drag region does not eat presses: NSH's own hit-testing returns "not a
  drag" at the nearest view that has a press handler, so 深色模式 and the connection chip in that
  header still take the click.

The desktop commands are not recorded by the headless engine, so a test cannot assert that the
bar is hidden — only what getting the geometry wrong would cost.

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

### The same screens in a browser

`src/backend.rs` is the only module that decides where the backend lives. Every screen calls
`backend::` rather than `api::`, and the module has two arms per endpoint: a native process calls
`api` directly, exactly as it did before this module existed; a page posts to `/api/*` on
`packages/server`, because a browser cannot load a Go archive and the `api` functions under
`wasm32` are stubs that refuse.

One `backend!` table writes both arms, listing 32 endpoints in the same order as the
`endpoints!` macro in `packages/server/src/routes.rs`. Two properties are what make the split
safe:

- **The signature is identical on both sides, error type included.** `api` compiles for the
  browser, so `api::ApiError` exists there, and the wire error is decoded back into a real
  `ApiErrorCode` — a screen that matches on a code keeps matching when the same screen is served
  to a page. Nothing above this module knows which side it is on.
- **The wire keys come from the parameter names.** `stringify!($arg)` produces the JSON key, and
  the server reads arguments by the same rule, so the two ends agree by construction. An endpoint
  cannot be wired to one side only: adding a row to the table compiles into both arms.

Three calls are deliberately absent from the table: the TUN helper dispatch,
`cleanup_stale_system_proxy_sentinel` and `shutdown_native_runtime`. They belong to whichever
process starts, and a page cannot spawn a root helper or take a proxy down on the way out — so
each native entry point owns its own copy, `src/main.rs` for the desktop app and
`packages/server/src/main.rs:70,83,102` for a browser, which is why no screen had to change. The
deleted `packages/web/src/main.rs` dispatched the helper and cleared the sentinel too, and left
out the exit cleanup; the server has all three.

The page reaches the server with no `capabilities` entry in `nsh.json`, which is deliberate: an
app that declares none gets NSH's default access, whose `is_unrestricted` (`nsh-core`'s
`access.rs`) is what lets the engine send the request without parsing the URL at all
(`nsh-engine`'s `request`), so it leaves as the relative `/api/…` the table wrote and the browser
resolves it against the origin that served the page. Same origin is also what the server's own
guard requires. The symbols are named rather than the line numbers because both files move on
every NSH release; `scripts/e2e-web.sh` is the check that the behavior is still what this
paragraph claims — it drives a real Chrome against a real server and fails if a request is
refused.

Verified in a real headless Chrome, not only by unit test: the overview renders the server's
answers — the core's `EMBEDDED · 1.13.14`, a `发现新版本 v1.0.13` banner from the update
check, this machine's actual system-proxy setting — and a write made from inside the page
round-trips (`POST /api/profile`, read back, `profile.json` in the server's
`KITTY_PROFILE_DIR`) with the session cookie attached. No console errors.

What this does *not* cover: the 测试连通 route is exercised against a server whose core is not
running, and the suite still never starts a core, so `measure_node_latency` has no test on either
arm. A page-driven connect has never been clicked.

### The same screens on a phone

`scripts/build-android.sh` builds the APK. It is a shell script rather than a Gradle project
because NSH already assembles APKs — `nsh package --target android` compiles this crate into
`libnsh.so` and signs a package with `aapt2`, `zipalign` and `apksigner`, no Gradle in sight —
and all that was missing was the two things that script cannot know about:

- **Our `VpnService`.** NSH's manifest template has one activity and no service, and its runtime
  is a prebuilt AAR whose manifest is pinned to `com.longbridge.nsh`, so there is no supported way
  to add a service, a permission, or `BIND_VPN_SERVICE`. The script takes `libnsh.so` out of the
  package NSH made and builds the real APK from `packages/mobile/android/AndroidManifest.xml`.
- **`libkitty_singbox.so`.** `packages/singbox/build.rs` builds the Go core `-buildmode
  c-shared` for Android, so `libnsh.so` names it in its `NEEDED` entries — and NSH copies only the
  one library it built. An APK from `nsh package` alone does not start, which was checked by
  installing one: `java.lang.UnsatisfiedLinkError: dlopen failed: library "libkitty_singbox.so"
  not found: needed by …/libnsh.so`, thrown out of `Native.<clinit>` the first time a view is
  built. That is why the script builds `-p singbox` per ABI and stores both libraries.

The app is the *same* Rust. `api`, `proxy-core` and `singbox` already carried 74
`target_os = "android"` branches from the Dioxus mobile shell, and they compile unchanged;
`backend.rs` is not involved, because a phone is a native process and calls `api` directly exactly
as the desktop does.

That last sentence used to be followed by "no screen changed, and `store.rs` does not know it is on
a phone". It was true of the port and it stopped being true with the three phone-only rules below,
so it is recorded here as the exception rather than as the rule. What `store.rs` does *not* do is
keep a second copy of any state for a phone: it asks one predicate, and every one of those
predicates is a free function in `domain.rs` that takes the platform as an argument, so a desktop
test binary asserts a phone's answer. That is the shape `update_channel_label` already used, and
the reason has not changed — `cfg!` is fixed by the binary that runs the test.

**A phone owns its TUN, and the interface has to stop pretending otherwise.** `VpnService` hands
the core its interface the moment the tunnel is approved, and `api::toggle_native_core` sets
`request.tun` itself, because a loopback listener alone would route nothing on a phone. So the TUN
row is not drawn there, and — the part that was a bug rather than a wart — neither is its
pre-flight: `store::toggle_core` asked the desktop's privileged helper on a phone whenever the
profile held `tun_enabled: true`, and `prepare_native_tun_mode` answers that platform with an
error, so 启动代理 failed with 准备 TUN 权限失败，内核未启动 and started nothing. The preference is
persisted and synced, which is why `domain::asks_for_tun_helper` decides it where the decision is
made: hiding the row alone would have left every phone that syncs from a desktop broken by a switch
it could not see. The rules page's 进程分流需开启 TUN note follows the same reasoning — on a phone
it would order the reader to enable a mode that is always on, through a switch that no longer
exists.

**A card is withheld when the platform can fill none of it.** 启动与终端 drew its heading above a
login-item switch that `Autostart::is_supported` had already (correctly) refused to draw, because a
phone has no login item to write — an empty box with a title. `SettingsSection::available` gates
the card on `autostart::SUPPORTED`, the same flag the row obeys, so the two cannot disagree; it is
not `!is_phone()`, because a web build is no phone and has no login item either, and gating on the
platform would have left the empty card on that page instead. `SettingsSection::visible` is the one
question the page and the search both ask, which is what keeps the search box from offering a
control the page will not draw — the reason 接管方式 loses the words `tun`, `系统代理` and `权限`
on a phone, where none of those three has a control any more.

Three things about the Java half are worth knowing before touching it:

- **The core's callbacks go through the application, not the activity.**
  `packages/singbox/src/android.rs` reaches Java through `ndk_context`, which `ndk-glue` normally
  fills in before `main` — and nothing runs `ndk-glue` here, because this is a `cdylib` an ordinary
  activity loads. `KittyApplication.attachBaseContext` calls `KittyVpnBridge.attach`, which hands
  the application context to Rust once. It has to be the application: `ndk-context` holds one
  object for the life of the process and cannot be re-pointed, while the activity changes on every
  rotation.
- **The VPN consent dialog has an activity of its own.** It is a result an activity must receive,
  and the activity on screen is `com.longbridge.nsh.AppActivity`, which is `final` and does not
  forward `onActivityResult`. A prebuilt runtime cannot be subclassed, so
  `KittyVpnPermissionActivity` is an invisible window that launches the system intent, owns the
  result, and starts the service. The config travels in its intent rather than in a static, so a
  process Android killed while the dialog was up resumes with it.
- **Fonts need nothing.** The three woff2 faces `app_root()` embeds are unwrapped by
  `nsh_core::text::font_data` before any platform sees them (`core/mod.rs:1683`), so Android's
  `ttf_parser`, which cannot read woff2, is handed a TrueType collection it can.

`extractNativeLibs` is true and the libraries are stored uncompressed and page-aligned. That is
what lets the loader resolve `libkitty_singbox.so` out of the archive; NSH's own template turns it
off, and can, because it ships a single library loaded by name.

**Verified on an Android 15 emulator** (API 35, `google_apis/arm64-v8a`, installed with
`adb install -g` and driven with `adb shell input`): the app launches, `libnsh.so` loads
(`nativeloader: Load …/libnsh.so … : ok`, which also proves `libkitty_singbox.so` resolved), and the
screens render — 总览, 节点 and 订阅 all read from a real profile. Importing a `ss://` link
parse-checks through `proxy-core` (it rejected a mistyped port with 订阅解析失败：链接格式无效:
invalid port number, then accepted a correct one), the node appears on 节点 with its address and
protocol, and 启动代理 reaches the system VPN dialog: `com.android.vpndialogs/.ConfirmDialog` came up
over `KittyVpnPermissionActivity`, and approving it created `tun0` at `172.19.0.1/30`, after which
the overview read 已连接 / 11 个活动连接 正在转发 with traffic in the graph. `KittyDnsBridge` logged
its adapter listening on a loopback port.

Building it that way found three defects, each fixed here rather than worked around:

- **The app would not have started at all.** `KittyApplication` attached the core's Java context
  from `attachBaseContext`, where `getApplicationContext()` is still null — `LoadedApk` sets
  `mApplication` *after* `Application.attach` returns — so the process died in
  `Unable to instantiate application … getApplicationContext(...) must not be null`. It attaches in
  `onCreate` now, which is still before NSH's activity builds its view.
- **A blank window, silently, from this repository's own manifest.** `AppActivity.rustApp()` reads
  `com.longbridge.nsh.RUST` through `getApplicationInfo().metaData`, which carries application-level
  meta-data only. The hand-written `AndroidManifest.xml` declared it on the `<activity>`, the read
  found nothing, and the app fell back to loading a JS bundle: `cannot read the app bundle:
  java.io.FileNotFoundException: nsh/bundle.nshc`, on an empty screen. NSH's generated manifest gets
  this right — checked against the one `nsh package` built for this app — and the hand-written one
  now matches it. The failure mode is what makes it worth writing down: nothing throws, no test
  covers a manifest, and the only symptom is a window with nothing in it.
- **The header sat under the status bar**, and the first card began a status bar's height too low,
  because the top safe-area inset was being spent in the body rather than the header. And the
  添加订阅 sheet kept 取消 and 导入 under the keyboard, so importing one needed the keyboard
  dismissed first. `the_status_bar_inset_is_spent_once` pins the first; the 添加订阅 sheet pads
  itself by `pb_keyboard()` for the second. The process picker's sheet is *not* padded, because
  nothing can raise a keyboard over it: its entry point is behind `process_rules_supported`, false
  on Android and iOS, while a desktop sets `keyboard_height` to 0 (`system.rs:279`) and the web
  runtime initialises it to 0 and never updates it.

**Not verified:** a physical phone, and `armeabi-v7a` at runtime — the 32-bit ABI compiles and its
library satisfies every symbol `libnsh.so` asks for, but the emulator image is arm64. Not verified
either: DNS resolution *through* the tunnel (the adapter started and the core reported forwarding;
no query was captured on the other side), latency measurement, cloud sync, and the traffic the core
forwards was the emulator's own, not a proxied request to a reachable node — the test node is
`198.51.100.10`, a documentation address.

## The visual language

It is carried over rather than redesigned. `theme.json` defines 45 roles
per appearance and keeps every color and radius the deleted Dioxus stylesheet (`app.css`) used,
at its original value, including the `rgba()` alphas the cards rely on. Three of those roles exist because that CSS had
them and the first port dropped them: `--switch-thumb`, the thumb's opaque fill,
`--switch-thumb-shadow`, the shadow that is the only thing separating a white thumb from an 82%
white track, and `--accent-glow`, the floating action's own shadow colour while it is lit. Where the CSS file also keeps a type scale, font weights, spacing steps and layout
widths as custom properties, NSH has those as typed styles instead, so they are written as
methods (`.text_xs()`, `px(264.0)`) rather than as tokens.

A control whose look is made of parts needs those parts styled at the right level, and this is
where the port's first pass went wrong: `checked_style` paints a `switch`'s own box, not its
track, so the checked fill drew an orange rectangle around a track that had never been sized.
`ui::controls::switch_state` now styles the track and the thumb themselves, takes their sizes
from `domain::SwitchGeometry` (the same numbers the old `app.css` gave `.switch` and `.switch::after`,
with the thumb's travel derived so it cannot stop matching the inset), and the box paints
nothing at all.

The same lesson in a smaller place: `controls::badge` sets no cross-axis alignment of its own, so a
pill sits wherever the row holding it puts one — which for every row in this app is centred. It used
to carry `.self_start()`, and inside a row that silently overrode the row's `items_center` and
pinned the pill to the top edge: the action pill on a two-line rule row drew 8.5pt above the row's
centre. `the_routing_badge_sits_level_with_its_row` measures that in points, and it is the case that
pins the fix; the DNS row's identical-looking pill sits inside a grouping wrapper that absorbs a bad
`align_self`, so its test cannot see this bug and says so.

One caller does have to know something: a pill's natural cross-axis size is *stretch*, so dropped
into a fixed-width **column** it fills it. The log row's 52pt decision lane is the only such slot,
and it writes `.items_start()` — measured on the engine, the pill is 36pt wide with it and 52pt
without. No test covers that lane, because a log entry only arrives from a core that is really
running; the line is deliberate, not the leftover of the bug.

The three variable faces (Space Grotesk, Inter, JetBrains Mono) are the woff2 bytes this crate
serves from `assets/fonts/`, the same faces the deleted Dioxus stylesheet inlined. CJK falls
through to the system font.

## What is still missing

Checked by reading both, not by assumption. What is left:

| Missing | What it means |
| --- | --- |
| Tray, close-to-tray | The menu-bar item and the "closing the window keeps the core running" behaviour. The deleted Dioxus desktop shell had both; this crate has neither. It is now purely app-side work: NSH 0.5 added `set_tray`/`tray()` (`nsh-core`'s `element::desktop`) with `.icon`, `.tooltip`, `.menu` and `.on_click`, backed by a real `NSStatusItem` in `nsh_platform_macos::tray`, and it binds the menu callbacks to the calling component's `Owner` itself — so the hop this row used to say had to be hand-built (AppKit's main thread into the engine thread, over a reactive graph that is not `Send`) is NSH's problem now, not ours. `AppOptions::keep_alive_on_close` still supplies the other half: it hides the window and keeps the engine, which the Dock restores. What is left is the decisions and the wiring — what the menu offers (it wants `AppStore::copy_proxy_environment`, and a quit that runs `api::shutdown_native_runtime`), whether a close hides or quits by default, and where the tray is declared so it exists for the app's life rather than one screen's. |
| 复制代理环境变量 | The user asked this off the settings page ("只需要在托盘中有就行"), so it is on no screen now: `AppStore::copy_proxy_environment` exists and nothing calls it until the tray does. Kept as its own row so a reader looking for the button finds the reason rather than an absence. |
| iOS | No build, and nothing to compare the reason against. `packages/singbox/build.rs:30` panics for any target OS outside `{android, macos, linux, windows}`, so the core cannot be compiled, and NetworkExtension — the only way an app that is not a `VpnService` may carry traffic on iOS, since Apple gives third-party apps no TUN — was never implemented. Android got a build without a Gradle shell; iOS needs a Swift one, which is a larger question than a build script. |
| Exit cleanup in a **packaged** build | Fixed for a quit, still open for a crash. NSH 0.5 added `on_app_quit`, which `process::register_exit_cleanup` takes from the root component — the one thing every runner mounts, `nsh package`'s generated one included — so 退出 now clears the system proxy in a release build. What no hook can reach is a process that never quits: `kill -9`, a crash, a power loss. That case still falls to the sentinel, which is why the next start runs `cleanup_stale_system_proxy_sentinel` rather than trusting the last exit. [Why the other two entry-point calls are fixed and this one is not](#process-entry-points-in-a-packaged-build). |

### Process entry points in a packaged build

Three calls belong to whichever entry point starts the process, and both deleted shells used
to own them — `packages/desktop/src/main.rs` and, as it turns out, `packages/web/src/main.rs`
too, which is worth remembering when the web server is restored, because it means the server
was a full privileged entry point, not only a UI host: dispatching the privileged TUN helper
(`api::run_tun_helper_from_args`), clearing a system proxy left dangling by a run that died
(`api::cleanup_stale_system_proxy_sentinel`), and taking the proxy and the core down on exit
(`api::shutdown_native_runtime`). `cargo run --features desktop` therefore behaves.

They now live in `src/process.rs`, and `native()` — the `#[native_module]` entry every runner
NSH generates evaluates before its `main` body, on macOS, Linux and Windows alike — calls
`process::boot` first. So do `src/main.rs`'s own entry points, which is why they no longer
duplicate the three calls.

That mattered because `nsh package` does not compile our `main.rs`: it generates and builds
its own runner (`.nsh/rust/<target>-app/main.rs`), whose `main` calls only `set_ui_sets(..)`
and the platform's `main()`. The one that broke rather than degraded was the helper. TUN runs
the core in *this same executable* re-launched with `--kitty-pro-tun-helper` and
administrator rights, so a packaged helper skipped the dispatch, opened a second GUI as root
and never answered the parent's RPC. Measured before the fix: the packaged binary, launched
with the helper flag, printed eleven font-fallback warnings — it was drawing the interface.
Measured after: `invalid TUN helper arguments` and exit 1 with no font line, while an
ordinary launch still draws.

Two things the hook forces:

- `nsh test`'s runner calls the same entry (`.nsh/rust/test/main.rs`), and the boot cleanup
  reads a sentinel scoped by `KITTY_PROFILE_DIR` but *writes* the machine's **real** system
  proxy, so `cleanup_is_ours` skips it whenever that variable is set. Pinned by
  `a_scratch_profile_keeps_the_boot_cleanup_off_the_machine`. The helper dispatch is safe for
  free — the flag is only ever present in the helper.
- The helper dispatch has to `process::exit` from a function whose stated job is registering
  components, which is why it is `boot()` and not three calls inline in `native()`.

Exit cleanup found its hook, but not at the runner: `AppOptions` still carries no callback
field (`trace`, `asset_base`, `chunks`, `app_id`, `title`, `size`, `display`, `keep_size`, …),
so `nsh package`'s `main` remains a place this crate cannot put code. NSH 0.5 put the hook
inside the app instead — `nsh_core::element::desktop::on_app_quit` — and it belongs to the
owner that registers it, which makes the root component the only thing every runner is
guaranteed to mount. `process::register_exit_cleanup` registers there.

Two things the hook's budget forced. It clears the proxy through
`api::disable_managed_system_proxy_before_core_outage` and does **not** call
`api::shutdown_native_runtime`: that one takes `core_slot`, `restart_native_core` holds that
lock across the bind retry window and the helper authorization wait, and `Engine::drop`
joins the engine thread with no deadline once the quit budget is spent — quitting during a
restart would have hung the app instead of cleaning it up. And the work is synchronous,
because the engine waits only `QUIT_TIMEOUT` (200ms) and quits regardless: nine
`networksetup` spawns measured at 110–140ms fit, but with under 100ms to spare, which is
why the sentinel stays as the fallback for a run cut off mid-write. Stopping the core is
not lost by leaving it out — the core is embedded, so process exit stops it either way.

`process::shut_down` is kept for `src/main.rs`, which is still the only entry point NSH lets
this crate write. The guard that keeps both off a test run is `cleanup_is_ours`, the same
predicate, for the same reason: what the cleanup writes is the machine's real proxy.

Every screen is ported and every settings card has a home here. The rows above are what still
separates this app from what the deleted shells shipped, and none of them is a decision this
crate can make on its own.

One card is distributed rather than copied. The original groups the core's version, its state, the
unavailable-core diagnosis and its retry under one 内核 heading; here those four things live where
each is acted on — the state in the header chip and the about card, the diagnosis and 重新检测内核
on 总览 beside the control they explain, and the override that replaces the config on 设置. A native
window has no scroll budget to spend on a card the user reads once.

### Diagnostics, about and the process picker: how they differ from the original

Both are ported, each in the form this shell can honestly offer.

The about card reports the figures the deleted Dioxus one reported, and `复制诊断信息` produces
the same text. Two things differ. The platform is derived from NSH's own idiom (`Desktop`/`Mobile`) rather
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

The card holds the same fields as the original: the provider segment, one address field per
provider, the object path with the bundle directory it implies, the credentials, and the four
actions (保存, 下载覆盖本机, 上传, 覆盖远端). `domain::SyncProviderForm::fields` is the single list
of which label edits which stored field, which is what stops the two providers' addresses from ever
being saved into each other's slot. Conflict handling, the plain-language translation of ETag and
baseline refusals, the two-step confirm with its expiry, and the pre-pull backup are all the
backend's, reused as they are.

Six fields for WebDAV and eight for S3 is a wall on a card that also carries a badge, a switch, a
note and four buttons, so the target's fields sit behind one row — 备份位置 — which the user asked
for. What makes a fold honest is its closed state, so that row carries
`domain::sync_target_summary`: the provider, the host, the object path, and the missing field when
there is one, in the same words `sync_target_incomplete` uses to park the buttons. The two can
therefore never disagree, and nothing secret is in the line — a host and a path name a server, and
no credential appears in it. The fold also opens itself when the target holds nothing but the
defaults the app wrote: 尚未填写 is not a summary of anything, and a closed fold over an empty form
is a wall with a caret on it. `SyncConfig::default()` ships `path: "kitty-pro-sync.json"`, which is
why "untouched" is a question about the address and credentials rather than about the path.

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

The fake remote's listener is nonblocking so its accept loop can notice `stop`, and on macOS that
flag reaches every socket it accepts, so the handler reads `WouldBlock` instead of waiting for a
request that arrives in pieces. Each accepted socket is therefore put back to blocking where it is
handed to a handler — in the fake remote and in the TUN helper alike, which fails the same way and
is why 启动 TUN was intermittently reporting 连接失败. `accept(2)` says the flag does not carry over
on Linux and tells portable code to set what it wants either way; nothing here assumes it does.

### App updates

The card reads the same state file the deleted Dioxus shell wrote, from the same three calls, and the
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
here, and a flat list in the core: that is a decision in
`proxy-core`'s parser, not a gap in this front end, and no screen in this crate can offer a
group the engine does not build. Closing it for real means teaching the parser to carry
provider groups into the config, which is a change to the shared backend.

`nsh.json` carries `icon` (a copy of `assets/icon/kitty-pro-1024.png`, which is what
`nsh package` wants: a square PNG), version and id, so a packaged build is named, versioned
and icons correctly without flags.

## Tests

`cargo test -p kitty-pro` runs the screens under NSH's real engine. Three things are
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
  `AppProfile` itself, so a pass means the screens read what the backend writes.

`every_status_pair_clears_aa_contrast` reads `theme.json` and checks every status color
against its own background in both appearances, so a token added later is covered by the
same rule that catches the existing ones.

The suite deliberately never presses 启动代理. That call starts the real sing-box core, which
hangs a test run and leaves a live proxy behind; the connect path is covered up to the guard
(`connect_is_off_until_there_is_something_to_connect_to`, `a_refused_press_starts_nothing`)
and no further. The same rule keeps 测试连通 unpressed (over a running core it is a real
outbound request), and 登录时启动 with it — that press writes a launch entry into whoever's
account runs the tests. Each is asserted as far as it can be without the side effect — present,
named, settled, and unreachable in the state where a press would cost something. 复制诊断信息 is
the exception, and the reason is in the section above: NSH's clipboard belongs to the test, so
that one can be pressed and read back.

复制代理环境变量 is no longer on that list because it is no longer on screen. The user asked for
it off the settings page; `AppStore::copy_proxy_environment` stays, waiting for the menu-bar item
that should own the action (see 托盘 in the gap list). While it was on the page its note claimed
the copied line pointed at a 局域网可达 address when 允许局域网连接 was on — the backend builds
that line from `SYSTEM_PROXY_HOST`, a constant `127.0.0.1`, so the note was false in the one case
it existed to describe. Nothing else in either shell repeats that claim, which is why it left with
the button rather than being corrected.

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
`cargo run -p kitty-pro --features desktop` and check both appearances and a phone-width
window. The user's screenshot of a broken toggle is what started this: the suite was green at the
time, so "green" means "no test looked at it".

The live half of the group switch has the same problem in miniature. `select_proxy_group`
against a running core is the path that avoids a restart, and the suite starts no core, so what
is covered is the saved-and-reported branch, the guards, and the persistence; the
switch-without-restart behaviour is reasoned from `api::select_proxy_group` and
`singbox::select_outbound` rather than observed. It is worth watching once by hand.

The frozen 运行时长 is the same shape again, and the diagnosis is the one thing here no test drove
end to end. `domain::traffic_poll_tick` is tested directly, including the tick that used to kill
the poll, and `domain::uptime_since` is tested against the clock. What is reasoned rather than
observed is the Android half: that a read refused while `VpnService` is raising the tunnel comes
back as the bridge's error *text* and fails to parse, so the poll died on its own first tick. That
chain is read out of `KittyVpnBridge.traffic()`, `KittyVpnService.trafficPayload()`,
`singbox::android::traffic` and `bridge/android_bridge.go`, and the screenshot — `00:00` under a
card saying 运行中, with the chart flat — is the symptom it predicts exactly. No device or
emulator is attached to this machine and the suite starts no core, so it was never reproduced
here. It is worth confirming on the phone: 启动代理, then watch 运行时长 climb and the bars move.
