# OpenXplorer native (Rust + GTK4)

The next OpenXplorer: the same Explorer skin, drawn with native GTK4 widgets
instead of an HTML page in WebKit. It replaces the Python/WebKit app in
`desktop/` once it reaches parity; until then it is a preview that runs side by
side with it.

The rewrite removes the HTML-to-Python command bridge and uses GTK's native
models, selection, menus, fonts and scaling. The toolkit also supplies the
building blocks for clipboard, drag-and-drop and accessibility; their complete
application workflows still need implementation and acceptance testing. Startup
and interaction performance must be measured before claiming an improvement.

## Layout

| Path | Responsibility |
|---|---|
| `crates/ox-core` | Toolkit-independent core: locations, settings, entries, places, clipboard formats and the transfer engine, all on GIO. No GTK. |
| `crates/ox-app` | The GTK4 application (`openxplorer-native`). |

The Python modules in `desktop/` are the behavioural specification. Each Rust
module names the Python file it ports; port its tests along with it.

## Build and run

Needs Rust 1.92+ (the minimum required by the locked GTK/GIO crates), GTK
4.14 development files, and Python 3 for compatibility tests:

```sh
sudo apt install libgtk-4-dev
cargo build --release --locked --manifest-path native/Cargo.toml
./native/target/release/openxplorer-native
```

The preview uses the application ID `io.winspace.Development.Native`, so it
never talks to a running Python OpenXplorer. It shares
`~/.config/winspace/settings.json` using the Python app's locking protocol.

## Checks

Run the check driver from the repository root:

```sh
sudo apt install libgtk-4-dev xvfb xauth dbus-x11
python3 native/tools/check.py
```

The driver checks inventory consistency, formatting and strict Clippy, compiles
every test target, then runs the actual test binaries on Xvfb with private D-Bus
sessions and disposable home/config/cache directories. It never connects tests
to the user's desktop or remote volume monitors. The hosted CI workflow uses
the minimum supported Rust version and the same driver. Its result is native
GTK/GIO **local** validation; simulated MTP tests do not certify phone hardware,
and no SMB server is exercised by these checks.

`python3 native/parity/check.py --require-replacement --gate replace --gate dolphin`
deliberately fails while bridge operations, existing OpenXplorer behaviours or
Dolphin must-haves still lack native verification. `parity/features.toml` lists
every behaviour the native app must provide; [parity/README.md](parity/README.md)
explains how a feature is marked done. See [ROADMAP.md](ROADMAP.md) for the
manual acceptance work that local tests cannot cover. The Python application
remains the shipped desktop while this preview is incomplete.

The [browsing milestone validation record](VALIDATION.md) lists the local checks
actually run and their limitations.

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
