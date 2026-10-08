# Contributing to OpenXplorer

## Before a change

Read [README.md](README.md), [AGENTS.md](AGENTS.md), [native/README.md](native/README.md), [the development guide](docs/development.md) and [SECURITY.md](SECURITY.md). Keep a change focused and explain the observable problem before changing implementation details. A visual improvement must preserve file-operation and desktop integration safeguards.

## Local development

The app is the Rust and GTK 4 program in `native/` (crates `ox-core` for the GIO services and `ox-app` for the interface). It needs Rust 1.92 or newer (`rust-version` in `native/Cargo.toml`), GTK 4.14 or newer, SQLite and libsoup 3 development files, and Python 3.11+ for the check driver and parity tools. The distribution packages below are the ones CI installs in `native/packaging/ci/prepare-container.sh`, including the test tools:

| Distribution | Packages |
|---|---|
| Ubuntu 24.04+, Zorin OS 18, Debian 13 | `build-essential pkg-config libgtk-4-dev libsqlite3-dev libsoup-3.0-dev xvfb xauth dbus-x11 gvfs gvfs-backends python3-gi gir1.2-glib-2.0 gnome-keyring gir1.2-secret-1 nodejs` |
| Fedora | `gcc pkgconf-pkg-config 'pkgconfig(gtk4)' 'pkgconfig(sqlite3)' 'pkgconfig(libsoup-3.0)' xvfb-run xorg-x11-server-Xvfb xauth gvfs gvfs-smb python3-gobject gnome-keyring libsecret nodejs` |
| openSUSE Tumbleweed | `gcc pkgconf-pkg-config 'pkgconfig(gtk4)' 'pkgconfig(sqlite3)' 'pkgconfig(libsoup-3.0)' glib2-tools xvfb-run xorg-x11-server-Xvfb xauth dbus-1 gvfs gvfs-backends gvfs-backend-samba python3-gobject typelib-1_0-Secret-1 gnome-keyring nodejs` |
| Arch Linux | `base-devel rustup gtk4 sqlite libsoup3 xorg-server-xvfb xorg-xauth gvfs gvfs-smb python-gobject gnome-keyring libsecret nodejs` |

Install Rust with rustup, then build and run the development build:

```sh
cargo build --locked --manifest-path native/Cargo.toml
./native/target/debug/openxplorer-native
```

Do not run the file manager as root. Node and pnpm are website and test tooling; the installed app does not depend on them. For the website, use Node.js 22.13+ and the repository's pinned pnpm version:

```sh
corepack enable
corepack prepare pnpm@10.34.5 --activate
pnpm install --frozen-lockfile
pnpm dev
```

Commit dependency changes together with the updated `Cargo.lock` or `pnpm-lock.yaml`. After a `Cargo.lock` change, regenerate the Flatpak's crate list with `python3 native/tools/flatpak_cargo_sources.py`. Do not regenerate a lock merely to bypass an installation failure. Keep local environment values and Cloudflare credentials outside source control.

## Pull requests

Create a feature branch and open a pull request into `main`. Direct pushes, force pushes and deletion of `main` are blocked, including for administrators. A second reviewer is not required.

Pull requests are squash-merged under their title, so the title must follow [Conventional Commits](https://www.conventionalcommits.org/) as `type(scope): description`, for example `fix(dialogs): Keep an Open dialog answerable until it closes`. The type is one of `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore` or `revert`; the scope names the area changed, such as `dialogs`, `transfers`, `web` or `deps`, and `!` after it marks a breaking change. The required **PR title** check enforces this, and runs again when you edit the title.

The description must name the model that generated the pull request on a line of its own, such as `Model: claude-opus-5-5`, or `Model: none` for a pull request written without one. The required **PR model** check enforces this. Dependabot pull requests are exempt.

## Checks before a pull request

Run the checks that cover the change and include their actual outcomes:

```sh
cargo fmt --manifest-path native/Cargo.toml --all -- --check
cargo clippy --manifest-path native/Cargo.toml --workspace --all-targets --locked -- -D warnings \
  -W clippy::pedantic -A clippy::module_name_repetitions -A clippy::must_use_candidate -A clippy::similar_names
python3 native/tools/check.py
python3 native/parity/check.py
```

`native/rustfmt.toml` sets a line width of 110. `native/tools/check.py` runs the parity inventory checks, the guard against icons drawn in code, rustfmt and Clippy with the workspace lints, and then every test executable and the doctests, each on its own Xvfb display with a private D-Bus session and disposable home, config, cache and runtime directories.

Tests must never reach the live desktop, the real home folder or real user data: do not run GTK tests on your own display, keep `DISPLAY` and `WAYLAND_DISPLAY` unset, and write only inside temporary directories. To run a single test while iterating, wrap it the same way, for example `env -u DISPLAY -u WAYLAND_DISPLAY dbus-run-session -- xvfb-run -a cargo test -p ox-app <filter>` with `HOME` and the XDG directories pointing into a temporary folder. Tests must not mount or open remote locations; transfer tests use simulated devices.

`native/parity/features.toml` lists every behaviour the app must provide, with its native status. A test that proves an item carries a `/// parity: ID` doc line; set the item to `done` and update its `native_note` only when it is implemented and a native test marked this way proves it. [native/parity/README.md](native/parity/README.md) explains the rules. `native/BACKLOG.md` lists the items still open after 2.0.0.

Build and verify a package without installing it (see [native/packaging/README.md](native/packaging/README.md) for RPM, Arch and Flatpak):

```sh
python3 native/tools/build_deb.py --app-id io.winspace.Development
python3 native/tools/verify_deb.py dist/native/<package>.deb
```

For website and repository changes, also run `pnpm check`, `pnpm build`, `pnpm security:source`, `python3 -m unittest discover -s tests -p 'test_release_source.py'` and `python3 -m unittest discover -s tests -p 'test_public_data.py'`. The other scripts in `tests/` drive Chromium with Playwright (see [apps/web/README.md](apps/web/README.md)).

GitHub checks use a read-only token. Pull requests run the native checks and the per-distribution package builds (Fedora, openSUSE Tumbleweed, Arch Linux, Ubuntu 24.04, Debian 13 and the Flatpak); the release and website jobs run only after a merge to `main`. A workflow file existing in the repository is not evidence that its hosted run passed.

The 1.x Python/GTK 3/WebKitGTK app has left the tree. Its last release is tag `v1.1.4`; its final sources, the behavioural specification that `native/parity/` cites as `v2.0.0:desktop/<file>`, are `desktop/` at tag `v2.0.0`. The native check driver extracts them from that tag for the compatibility tests (`native/tools/python_app.py`).

## File-operation changes

Use disposable directory trees and non-critical shares. Test collisions, cancellation, partial copies, offline servers, read-only snapshots and stale cached metadata. Never substitute permanent deletion when Trash is unsupported. Do not overwrite a live file during Previous versions restore. Keep passwords out of logs, fixtures, screenshots and source archives.

## UI and documentation

Follow [native/docs/ui-spec.md](native/docs/ui-spec.md) for the look. Use only the Fluent icon files bundled under `native/crates/ox-app/resources/icons`; never draw icons in code. Preserve keyboard access, accessible names and reduced-motion behaviour. For the website, change shared TSX/CSS sources, then regenerate designs; do not hand-edit generated pitches. `apps/web/lib/docs.json` is the canonical guide source. Regenerate its Markdown with `python3 tools/sync-docs.py`; use `pnpm designs` to rebuild standalone designs.

Use only fictional names, paths and shares in examples and screenshots, following [docs/PRIVACY.md](docs/PRIVACY.md). Use the capture scripts for public images (`python3 tools/capture-screenshots.py` and `python3 tools/capture-native-tour.py`, which picture the native app in an isolated session with a fictional demo tree), review every picture, then run `python3 tools/audit-public-data.py`. Keep private denylist files outside the repository. The default audit verifies packaging and provenance; private identifiers require the optional external denylist and visual review.

Keep documentation and release claims synchronized with behavior and tests. Generated installers, archives, previews, local logs and `test-results/` stay out of Git. Preserve the corresponding-source build tools and visible website source link; see the [public-release checklist](docs/PUBLIC-RELEASE-CHECKLIST.md).

## Compatibility and license

The `winspace` configuration paths, `io.winspace.Development` application/desktop identity and credential schemas are stable compatibility contracts. Propose an explicit migration before renaming them. Pinning a folder also adds a line to the desktop's `~/.config/gtk-3.0/bookmarks`; the app records the lines it added in `desktop-bookmarks` in its settings folder and removes only those. Preserve existing notices.

Contributions are submitted under AGPL-3.0-only unless a documented file-level exception applies. Preserve the root [LICENSE](LICENSE), [NOTICE](NOTICE), upstream [MIT notice](licenses/Winspace-MIT.txt), desktop copies and [third-party notices](THIRD_PARTY_NOTICES.md). Include provenance for copied code/assets and ensure you are authorized to contribute them. No CLA is required by this repository.

## Suggested pull-request summary

Describe the problem, the resulting behavior, tests actually run, known limitations and any migration impact. Use the pull-request template. For screenshots, use synthetic filenames and shares. If you did not run native tests, state that plainly. Report security-sensitive findings through the process in [SECURITY.md](SECURITY.md), without exposing credentials or private filesystem data in a public issue.
