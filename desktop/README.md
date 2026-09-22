# OpenXplorer desktop

**Windows File Explorer-inspired file manager for Linux.** Zorin OS is the primary target; compatible Ubuntu and Debian desktops are additional targets, not certified configurations.

Version **1.1.3** fixes dragging files with punctuation in their names and simplifies the update dialog; 1.0.1 added connected-device browsing and 1.0.0 was the first stable release, renamed from Winspace 0.7.0. The native engine is Python + GTK 3/WebKitGTK + GIO/GVfs. The website is separate; Node.js is not a desktop runtime dependency.

## This release

The native window immediately renders the app layout with static placeholders while the real interface starts. It uses the saved light/dark theme; startup recovery remains available if loading fails.

Use **Check for updates** at the bottom right beside the view controls to review, verify and install a stable GitHub release, then restart. Installation uses APT and system administrator approval; it never runs automatically. Older releases need one manual installation of 1.1.0 first.

Merge tabs into existing windows with native tab dragging or **Move tab to window…**.
ZIP opening is an independent optional association, not the folder default.
Connected phones exposed by GVfs now appear under This PC and can be browsed without mounting or indexing them automatically.
Copy and cut/paste conflicts now offer **Replace existing** or **Skip duplicates**. Phone copies no longer attempt unsupported Unix permission changes through MTP.
See [upgrade notes](UPGRADE.md) for installation and Brave diagnostics.

## Drag files between applications

Select files or folders and drag them into a compatible editor or attachment area.
Drop into an OpenXplorer folder or another window to confirm a copy; Quick access
drops pin folders. File drops are copy-only. Extract ZIP members first, and use an
already-mounted local path for network items when the receiving app requires one.
See the [interaction gap analysis](../docs/FILE-INTERACTION-GAPS.md) for limits and validation.

## ZIP files and larger text

Right-click a ZIP and choose **Extract all…**, then choose an existing destination and a **new** output-folder name. The ZIP is unchanged and existing files are not overwritten. A yellow folder-and-zipper icon distinguishes archives from ordinary folders. Double-click still opens the read-only ZIP browser.

Use **Ctrl +** (also Ctrl =), **Ctrl −**, and **Ctrl 0** to enlarge, reduce, or reset text. **Settings → Appearance & layout → Text size** provides the same saved 80%–200% setting.

## Install

Finish transfers and quit existing windows and the optional service with `openxplorer --quit` (`winspace --quit` for older Winspace installations). From the download directory:

```sh
sudo apt update
sudo apt install ./openxplorer_1.1.3_all.deb
openxplorer --check
openxplorer --restart
```

From this directory after building, use `./dist/openxplorer_1.1.3_all.deb` instead. Never run the file manager with sudo. The optional administrative mount helper is a separate command.

## Build from source

```sh
python3 tools/build_deb.py
python3 tools/verify_deb.py dist/openxplorer_1.1.3_all.deb
```

Debian/Ubuntu `dpkg-deb` is required. CairoSVG is required at build time for installer raster icons; it is not a runtime dependency. Runtime libraries are provided by APT, not vendored. No root privileges or dependency downloads are used by the package builder.

## Test

```sh
python3 -m unittest discover -s tests -p 'test_*.py'
node --test tests/type_select.test.cjs
python3 tools/build_preview.py
python3 tests/ui_release.py
```

Browser tests require Python Playwright and Chromium. Native GIO tests need distro-managed GI libraries. See `TEST-REPORT.md` for this release's actual results; browser simulation is not native validation.

## User documentation

- [Zorin and Brave integration](ZORIN-SETUP.md)
- [Complete retained feature manual](MANUAL.md)
- [1.1.0 upgrade notes](UPGRADE.md)
- [Source repository documentation](../docs/introduction.md)

## Compatibility contract

Public app name: **OpenXplorer**. Debian package and command: **openxplorer**. The package replaces `winspace-explorer (<< 0.8.0)` and keeps `winspace` / `winspace-mount-share` aliases.

The `io.winspace.Development` desktop/application ID, `~/.config/winspace`, cache paths, keyring schemas and internal bridge identifiers are intentionally retained. Existing user settings, default associations and explicit reveal integrations continue to refer to the same identity. Installation does not edit user defaults, credentials or network mounts.

## License

The modified application is licensed **AGPL-3.0-only**; see [LICENSE](LICENSE). No warranty. Original MIT attribution and terms are preserved in [licenses/Winspace-MIT.txt](licenses/Winspace-MIT.txt). Earlier Winspace releases remain available under their original terms. Distribution dependencies retain their own licenses.
