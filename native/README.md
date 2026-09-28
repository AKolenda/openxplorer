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
| `parity/` | What the native app must do: every behaviour (`features.toml`) and every Python bridge operation (`bridge.json`), with their checker. |
| `docs/ui-spec.md` | The visual specification: the current skin, refined toward Windows 11 File Explorer. |
| `tools/check.py` | The check driver described below. |

The Python modules in `desktop/` are the behavioural specification. Each Rust
module names the Python file it ports; port its tests along with it.

## Build and run

Needs Rust 1.92+ (the minimum required by the locked GTK/GIO crates), GTK
4.14 development files, and Python 3.11+ for the parity checks and the
compatibility tests:

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

The driver runs the parity inventory checks, its own tests, the guard against
icons drawn in code (see [Icons](#icons)), rustfmt and Clippy with the workspace
lints, and compiles every test target. It then runs each
test binary, and the doctests, on its own Xvfb display with a private D-Bus
session and disposable home, config, cache and runtime directories, so tests
never see the user's display, session bus, settings or remote volume monitors.
Each run starts in a new process session. When it finishes, fails or exceeds
`--test-timeout` (180 seconds by default), every process it started, including
Xvfb and the bus daemon, is stopped before its temporary directories are
deleted.

This isolates the desktop session, not the filesystem: tests can still reach
absolute paths, so they must write only inside temporary directories. GIO keeps
using GVfs, because the app relies on its `smb://` and `mtp://` handling. Tests
must not mount or do I/O on remote locations; transfer tests use simulated
devices.

The hosted CI workflow uses the minimum supported Rust version and the same
driver, and runs the release-source and public-data policy tests in a separate
job. Its result is native GTK/GIO **local** validation; simulated MTP tests do
not certify phone hardware, and no SMB server is exercised by these checks.

`python3 native/parity/check.py --require-replacement --gate replace --gate dolphin`
deliberately fails while bridge operations, existing OpenXplorer behaviours or
Dolphin must-haves still lack native verification. `parity/features.toml` lists
every behaviour the native app must provide; [parity/README.md](parity/README.md)
explains how a feature is marked done. See [ROADMAP.md](ROADMAP.md) for the
manual acceptance work that local tests cannot cover. The Python application
remains the shipped desktop while this preview is incomplete.

The [browsing milestone validation record](VALIDATION.md) lists the local checks
actually run and their limitations.

## Icons

Every icon is an unmodified file from Microsoft's MIT-licensed Fluent UI System
Icons or Fluent Emoji, vendored in `crates/ox-app/resources/icons/`.
[SOURCES.md](crates/ox-app/resources/icons/SOURCES.md) records each file's set,
version, upstream name and SHA-256, and a test checks the files against it; the
licence texts are in `licenses/` and `THIRD_PARTY_NOTICES.md`. `build.rs`
compiles them into the binary as a GResource (`glib-compile-resources` comes
with the GLib development files), and the app adds it to the display's icon
theme at startup.

- `src/icons/icon.rs` is the only place icon names live: code shows an
  `Icon`, never a name or a file. Names start with `ox-`, so a desktop icon
  theme cannot replace them.
- Monochrome glyphs are `-symbolic` icons, which GTK paints in the CSS `color`
  of their image, so they follow light, dark, hover and disabled states.
- Pictures made of several icons (the zip badge, the green network bar, the red
  cross of a disconnected share) are `ArtImage`s: real icons layered with
  `gtk::Overlay` and small boxes the skin colours (`resources/skin/icons.css`).
- To add an icon, copy the upstream file byte for byte under the `ox-` name,
  list it in `icons.gresource.xml` and `SOURCES.md`, and add an `Icon` variant.
  Never draw one: the check driver fails on SVG path data, GTK or Cairo drawing
  calls and pictures embedded in the code, stylesheets and templates, and on
  any image file under `crates/` outside `resources/icons/hicolor/`.

## Code standards

Code must be clean, readable and idiomatic. `python3 native/tools/check.py`
enforces formatting and the Rust lints; reviews enforce the rest.

Rust:

- `rustfmt` formatting (`rustfmt.toml`) and zero warnings from the workspace
  lints in `Cargo.toml`: Clippy's `all` and `pedantic` groups with three
  documented allowances, `missing_docs`, and no `unsafe` code. Fix a finding
  rather than silencing it; an `#[allow]` that is truly needed says why.
- Small modules with one responsibility; split a file before it passes about
  500 lines. No dense one-line logic: name intermediate values.
- No `unwrap()` outside tests; use `expect("why this holds")` for real
  invariants and return errors for everything else.
- Every public item has a doc comment saying what it is for, with `# Errors`
  and `# Panics` sections where they apply.
- Code and tests ported from Python say so: "Ported from `desktop/core.py`".
- Tests accompany behaviour. A test that proves an inventory feature carries a
  parity marker such as `// parity: NAV-001` (see
  [parity/README.md](parity/README.md)).
- The transfer engine is ported test-first from the Python suite and must keep
  every safety rule in `desktop/operations.py`.

Python tooling (`native/tools`, `native/parity`, and the repository tools and
tests the native work changes):

- PEP 8, with lines of at most 99 characters.
- Type hints on every function, and a docstring saying what it does and why.
- Functions of about 40 lines at most. No dense one-liners and no statements
  joined with semicolons.
- `pathlib` for paths, `argparse` with help text for command-line options, and
  error messages that say what failed and what to do about it.
- The standard library only.
- Tests named for the behaviour they prove, with `subTest` for named cases.

Everything else:

- User-facing text keeps the existing app's wording (see `desktop/ui/app.js`
  and `apps/web/lib/docs.json`).
- Each source file starts with an `SPDX-License-Identifier: AGPL-3.0-only`
  comment.
- CI workflows and documentation have clear step names and headings, comment
  every non-obvious choice, and make no claim that is no longer true.

## Compatibility contracts

Do not rename the `winspace` settings directory, the keyring schema
`io.winspace.SmbCredentials`, the MIME handlers or the final application ID
`io.winspace.Development` (see `AGENTS.md`). Keep every desktop integration
opt-in: installing or running the app must not change file-manager defaults,
browser profiles, folder locations or mounts.
