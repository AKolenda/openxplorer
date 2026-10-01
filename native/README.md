# OpenXplorer native (Rust + GTK4)

OpenXplorer since 2.0.0: the same Explorer skin, drawn with native GTK4
widgets instead of an HTML page in WebKit. It replaces the Python/WebKit app
of OpenXplorer 1.x, which is no longer released and has left the tree: its last
release is tag `v1.1.4`, and its final sources are `desktop/` at tag `v2.0.0`.
Releases ship the stable channel (`io.winspace.Development`, see
[packaging/README.md](packaging/README.md)); the parity items still open when
2.0.0 shipped are in [BACKLOG.md](BACKLOG.md). The preview channel below
remains for trying a development build beside an installed release.

The rewrite removes the HTML-to-Python command bridge and uses GTK's native
models, selection, menus, fonts and scaling. The toolkit also supplies the
building blocks for clipboard, drag-and-drop and accessibility; their complete
application workflows still need implementation and acceptance testing. Startup
and interaction performance must be measured before claiming an improvement.

## Layout

| Path | Responsibility |
|---|---|
| `crates/ox-core` | Toolkit-independent core: locations, settings, entries, places, clipboard formats, the transfer engine and file operations, and the backend services (search, network sign-in and mounts, ZIP archives, previous versions, folder sizes, desktop integration, updates and tab handoff), all on GIO. No GTK. The module table in `crates/ox-core/src/lib.rs` names the Python file each module ports. |
| `crates/ox-app` | The GTK4 application (`openxplorer-native`). |
| `parity/` | What the native app must do: every behaviour (`features.toml`) and every Python bridge operation (`bridge.json`), with their checker. |
| `docs/ui-spec.md` | The visual specification: the current skin, refined toward Windows 11 File Explorer. |
| `tools/check.py` | The check driver described below. |

The Python modules of the 1.x app are the behavioural specification. Each Rust
module names the Python file it ports, as `v2.0.0:desktop/<file>` (read it with
`git show v2.0.0:desktop/<file>`).

## Install

The preview installs beside the Python app, under its own application ID
`io.winspace.Development.Native`, and changes no default applications,
mounts or user data. Download the file for your distribution from the
release, then:

| Distribution | Install |
|---|---|
| Ubuntu 24.04 and newer, Zorin OS 18, Debian 13 and newer | `sudo apt install ./openxplorer-native_<version>_amd64.deb` |
| Fedora | `sudo dnf install ./openxplorer-native-<version>-1.<dist>.x86_64.rpm` |
| openSUSE Tumbleweed | `sudo zypper install ./openxplorer-native-<version>-1.<dist>.x86_64.rpm` |
| Arch Linux | `sudo pacman -U openxplorer-native-<version>-1-x86_64.pkg.tar.zst` |
| Debian 12 and any other distribution | `flatpak install --user io.winspace.Development.Native.flatpak` |

The distribution packages need GTK 4.14; Debian 12 has GTK 4.8, so use the
Flatpak there. The Flatpak keeps its settings in `~/.var/app/`, apart from a
distribution package's. [packaging/README.md](packaging/README.md) describes
every format, how to build it, its dependencies, the Flatpak's permissions
and how the release that replaces the Python app moves existing users over.

## Build and run

Needs Rust 1.92+ (the minimum required by the locked GTK/GIO crates), GTK
4.14, SQLite and libsoup 3 development files (libsoup is the updater's HTTPS
client), and Python 3.11+ for the parity checks and the compatibility
tests:

```sh
sudo apt install libgtk-4-dev libsqlite3-dev libsoup-3.0-dev
cargo build --release --locked --manifest-path native/Cargo.toml
./native/target/release/openxplorer-native
```

The preview uses the application ID `io.winspace.Development.Native`, so it
never talks to a running Python OpenXplorer. It shares
`~/.config/winspace/settings.json` using the Python app's locking protocol.

## Checks

Run the check driver from the repository root:

```sh
sudo apt install libgtk-4-dev libsqlite3-dev libsoup-3.0-dev xvfb xauth dbus-x11 gvfs gvfs-backends python3-gi gir1.2-glib-2.0 gnome-keyring gir1.2-secret-1
python3 native/tools/check.py
```

`gvfs` provides the `trash:///` backend the Recycle Bin and Undo tests use, and
`gvfs-backends` the `smb://` backend of the not-mounted share tests.
`python3-gi` lets the interoperability tests import the Python app's GIO
modules: the driver extracts `desktop/` of tag `v2.0.0` for the run
(`tools/python_app.py`, which fetches the tag from `origin` when a shallow
checkout lacks it) and names it in `OX_PYTHON_APP`. With `gnome-keyring` and `gir1.2-secret-1`, the keyring tests use a
disposable GNOME Keyring on the private bus and the Python app's libsecret
calls; without them they only check that the keyring is reported unavailable.
CI sets `OX_REQUIRE_KEYRING=1`, which makes a missing keyring fail them instead.

The driver runs the parity inventory checks, its own tests, the guard against
icons drawn in code (see [Icons](#icons)), rustfmt and Clippy with the workspace
lints, and compiles every test target. It then runs each
test binary, and the doctests, on its own Xvfb display with a private D-Bus
session and disposable home, config, cache and runtime directories, so tests
never see the user's display, session bus, settings or remote volume monitors.
Each run starts in a new process session. When it finishes, fails or exceeds
`--test-timeout` (600 seconds by default), every process it started, including
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
manual acceptance work that local tests cannot cover.

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
  A network location shows the same art everywhere: sidebar, cards, tabs and
  the details pane.
- Icons are hidden from screen readers, as app.js marks them `aria-hidden`;
  the control around an icon names it.
- The loading spinner is the one picture GTK takes from the desktop theme: it
  is an animation, and the vendored Fluent set has no spinner. GTK's own
  images, such as the search box's clear button, show bundled icons.
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
- A module with submodules is a `name.rs` file beside a `name/` directory,
  never `name/mod.rs`. The one exception is a helper folder under a crate's
  `tests/`, such as `tests/transfer_support/mod.rs`: Cargo would build a
  `tests/transfer_support.rs` as a test binary of its own.
- Items get the narrowest visibility they need. `ox-app` warns on
  `unreachable_pub`, so its crate-visible items say `pub(crate)`.
- No `unwrap()` outside tests; use `expect("why this holds")` for real
  invariants and return errors for everything else.
- Every public item has a doc comment saying what it is for, with `# Errors`
  and `# Panics` sections where they apply.
- Code and tests ported from Python say so: "Ported from `v2.0.0:desktop/core.py`".
- Tests accompany behaviour. A test that proves an inventory feature carries a
  parity marker in its doc comment, such as `/// parity: NAV-001` (see
  [parity/README.md](parity/README.md)).
- The transfer engine is ported test-first from the Python suite and must keep
  every safety rule in `v2.0.0:desktop/operations.py`.

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

- User-facing text keeps the 1.x app's wording (see `v2.0.0:desktop/ui/app.js`
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
