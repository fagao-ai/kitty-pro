You are an expert [Native Shell](https://nsh.dev) assistant. This app's interface is
NSH in Rust (`packages/nsh-app`). It used to be Dioxus 0.7; that front end and its
server functions have been deleted, so Dioxus APIs, `rsx!`, `Scope`, `use_resource`,
`ServerFnError` and `dx` belong to no part of this repository any more.

## Read these first

The two skills vendored under `skills/` are the requirement, not background reading.
`.agents/skills` symlinks them for tooling that looks there.

- `skills/nsh/SKILL.md` — reactivity, components, styling, testing, packaging, and the
  references it points at (`references/rust.md` above all).
- `skills/nsh-design-guides/SKILL.md` — the design guides. Read the guide file itself
  before any screen, layout, colour, density, interaction-state or copy change.

`packages/nsh-app/README.md` records this app's own rules: how the store and the
screens are split, why each non-obvious control is drawn the way it is, and what is
still missing. Read it before changing a screen, and update it when you change one.

## Shape of the code

```
packages/nsh-app     the interface: NSH in Rust, and the only front end
  src/domain.rs      the state's rules, testable without a renderer
  src/store.rs       one AppStore of signals; every backend call starts here
  src/shell.rs       window chrome: rail, pages, notices
  src/pages/         one module per page
  src/ui/            the controls and the theme
  tests/app.rs       the screens, driven headless through nsh-engine
packages/api         the backend: profile, core control, sync, updates, system proxy
packages/proxy-core  subscription parsing, sing-box JSON, the shared types
packages/singbox     the embedded sing-box core, a Go archive behind a C bridge
```

`api`, `proxy-core` and `singbox` are called directly, in one process. There is no
server, no HTTP, and no second binary. An api function is a plain `async fn`
returning `Result<T, api::ApiError>`; `ApiError` carries an `ApiErrorCode` and a
user-facing message, and `Display` renders `[Exxxx] message`.

## Commands

```sh
cargo run -p kitty-pro --features desktop   # a real window
cargo test -p kitty-pro                     # the screens, headless
nsh run                                         # hot reload (needs the nsh CLI)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`desktop` gates the windowing backends because NSH's Linux host needs GTK 4.14
development packages that CI does not carry. Everything else builds without them.

## House rules this codebase already follows

- Comments and documentation say why. A file that looks odd is usually odd because a
  bug got it that way; check `git log` before changing it.
- Behaviour belongs in `domain.rs` so a test can drive it without a window.
- Do not add a dependency to cover what NSH or the standard library already does.
- Copy is Chinese first, with English added through `src/i18n.rs`; a test sweeps the
  screens for untranslated strings.
