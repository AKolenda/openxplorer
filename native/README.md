# OpenXplorer native (Rust + GTK4)

The next OpenXplorer: the same Explorer skin, drawn with native GTK4 widgets
instead of an HTML page in WebKit. It replaces the Python/WebKit app in
`desktop/` once it reaches parity; until then it is a preview that runs side by
side with it.

Why: every WebKit window starts its own web engine (about 1.3–1.8 s to a usable
window even with the background service), menus are clipped to the window,
list accessibility is poor, and about 1,300 lines of Python plus 60 KB of
JavaScript exist only to bridge HTML to the desktop. A GTK4 window opens in
tens of milliseconds inside a running process and gets keyboard navigation,
rubber-band selection, drag and drop, clipboard, accessibility, fonts and
scaling from the toolkit.

## Layout

| Path | Responsibility |
|---|---|
| `crates/ox-core` | Toolkit-independent core: locations, settings, entries, places, clipboard formats and the transfer engine, all on GIO. No GTK. |
| `crates/ox-app` | The GTK4 application (`openxplorer-native`). |

The Python modules in `desktop/` are the behavioural specification. Each Rust
module names the Python file it ports; port its tests along with it.

## Build and run

Needs Rust 1.80+ and GTK 4.14 development files:

```sh
sudo apt install libgtk-4-dev
cargo build --release --manifest-path native/Cargo.toml
./native/target/release/openxplorer-native
```

The preview uses the application ID `io.winspace.Development.Native`, so it
never talks to a running Python OpenXplorer. It shares
`~/.config/winspace/settings.json` using the Python app's locking protocol.

## Checks

Run all of these before a change is done:

```sh
cd native
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Code standards

- `rustfmt` formatting (`rustfmt.toml`) and zero `clippy` warnings.
- Small modules with one responsibility; split a file before it passes about
  500 lines. No dense one-line logic: name intermediate values.
- No `unsafe`. No `unwrap()` outside tests; use `expect("why this holds")`
  for real invariants and return errors for everything else.
- Every public item has a doc comment saying what it is for.
- User-facing text keeps the existing app's wording (see `desktop/ui/app.js`
  and `apps/web/lib/docs.json`).
- Each source file starts with `// SPDX-License-Identifier: AGPL-3.0-only`.
- Tests accompany behaviour. The transfer engine is ported test-first from the
  Python suite and must keep every safety rule in `desktop/operations.py`.

## Compatibility contracts

Do not rename the `winspace` settings directory, the keyring schema
`io.winspace.SmbCredentials`, the MIME handlers or the final application ID
`io.winspace.Development` (see `AGENTS.md`). Keep every desktop integration
opt-in: installing or running the app must not change file-manager defaults,
browser profiles, folder locations or mounts.
