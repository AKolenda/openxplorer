# OpenXplorer 1.0.2 — replacement and phone-transfer maintenance build

## 2026-09-14 replacement/phone-transfer run

Executed for the 1.0.2 source and package. Older sections below are retained
records, not checks rerun for this maintenance build.

| Check | Result | Scope |
|---|---|---|
| Full Python regression suite | 558 passed | Desktop logic with disposable files and backend doubles, including staged Replace/Skip behavior, directory merging, rollback and phone-style filesystems without Unix mode support. |
| Explicit local GIO integration suite | 5 passed | Real GIO operations in a disposable local directory, including atomic replacement; not MTP or SMB. |
| JavaScript unit runner | 37 passed | Type-to-select helper behavior. |
| JavaScript syntax check | Passed | Final shared desktop UI source parsed by Node. |
| Focused replacement browser check | 4 passed | Shared UI in bundled Chromium: Replace and Skip choices, folder-merge explanation and dispatched replacement policy; simulated filesystem. |
| Website TypeScript check | Passed | Dependency-aware `pnpm check`. |
| Next.js production build | Passed | Compiled, typechecked and generated 21 static pages. |
| Documentation and design regeneration | Passed | Synchronized Markdown and standalone designs from shared sources; not a Next.js build. |
| Package build and verification | 52 passed | `openxplorer_1.0.2_all.deb`: metadata, payload, dependencies, checksums, syntax and source correspondence; no installation. |

No physical phone was connected to this build environment. The reported MTP
failure is covered with an MTP-style backend regression that exposes a local
FUSE path but rejects `chmod`; an actual Pixel/MTP replacement and copy still
need confirmation on the target machine. The full Python Playwright UI suite
was not run because that module is unavailable; the four focused checks used
the bundled Node Playwright runtime. No package was installed, no device was
mounted automatically and no website or release was uploaded.

---

# OpenXplorer 1.0.1 — connected-device maintenance build

## 2026-09-13 connected-device run

Executed for the 1.0.1 source and package. Older sections below are retained
records, not checks rerun for this maintenance build.

| Check | Result | Scope |
|---|---|---|
| Full Python regression suite | 550 passed | Desktop logic with disposable local files and explicit GIO volume/mount doubles, including MTP, gPhoto2 and AFC URI handling. |
| JavaScript unit runner | 37 passed | Type-to-select helper behavior. |
| JavaScript syntax check | Passed | Final shared desktop UI source parsed by Node. |
| Website TypeScript check | Passed | Dependency-aware `pnpm check`. |
| Focused connected-device browser check | 5 passed | Shared UI in bundled Chromium with mocked mounted/unmounted phones; no USB or native bridge. |
| Next.js production build | Passed | Compiled, typechecked and generated 21 static pages. |
| Documentation and design regeneration | Passed | Synchronized Markdown and standalone designs from shared sources; not a Next.js build. |
| Screenshot regeneration | 7 product stills | Actual shared HTML rendered in bundled Chromium with fictional fixtures; not native WebKit or a physical device. |
| Package build and verification | 52 passed | `openxplorer_1.0.1_all.deb`: metadata, payload, dependencies, checksums, syntax and source correspondence; no installation. |

The environment exposed GIO volume-monitor signals, but no physical phone was
connected, so USB/MTP/PTP/AFC discovery, unlock/trust prompts and transfers on a
real handset remain target-machine checks. The full Python Playwright UI suite
was not run because that module is unavailable; the five focused device checks
used the bundled Node Playwright runtime. No package was installed and no device
was mounted automatically.

---

# OpenXplorer 1.0.0 — executed checks

## 2026-09-13 stable 1.0.0 release run

Executed on Zorin OS 18.1 (Python 3.12, Node 24) for the 1.0.0 version change. The
sections below are retained records of earlier runs, not tests rerun today.

| Check run for this release | Result | Scope |
|---|---|---|
| Python regression suite | 542 passed | Desktop logic with disposable local files and explicit desktop-library doubles. |
| JavaScript unit runner | 74 passed | Selector, snapshot, text-size and helper tests. |
| Application browser suites | 364 passed | Actual application HTML in Chromium: release 68, type-to-select 61, v06 65, v07 53, mouse/middle-click 29, file-drag contract 30, terminal 19, two-window transfer contract 28, UI/native contract 11. Simulated filesystem and desktop transport. |
| Standalone website suite | 103 passed | Generated shared HTML, Markdown copying and relative links. |
| Production export browser suite | 47 passed | HTTP-served static export, hydration, docs label now reading `1.0.0`, canonical metadata. |
| Mobile/docs/date suite | 162 passed | Chromium narrow viewports; no physical-device claim. |
| Website TypeScript check | Passed | `pnpm check`, dependency-aware `tsc --noEmit`. |
| Next.js production build | Passed | `pnpm build`, 21 generated static pages. |
| Website syntax transpilation | 16 files passed | Syntax only. |
| Corresponding-source exclusion tests | 9 passed | Disposable trees, symlinks, credential/cache exclusions. |
| Project source security checker | 20 passed | Project invariants, not an independent audit. |
| Package build and verification | 52 passed | `openxplorer_1.0.0_all.deb`: control version `1.0.0`, stable AppStream release entry, packaged bytes matching source; no installation. |
| Public-data audit | Passed | Rebuilt installer, source archive and regenerated captures; no private denylist configured. |
| Screenshot regeneration | 7 product + 2 website stills | Recaptured from the actual UI so published images show `1.0.0`, not the candidate label. |

Not run here: native GTK/WebKit tab and file transport suites, which need an
isolated X server, plus native Zorin/Wayland, live SMB and installed-application
interoperability. GitHub Actions runs the GTK/WebKit transport jobs on the pull
request; the remaining items stay environment-specific checks for the installing
user, as listed in [the release checklist](../docs/RELEASE-CHECKLIST.md).

---

# File interaction update — executed checks

Date: 2026-09-11. This section covers the current drag/clipboard changes. The
September 7 report below is historical and is not a list of tests rerun today.

| Check run for this update | Result | Scope |
| --- | --- | --- |
| Full Python regression suite | 542 passed | Production logic with disposable local files and explicit test doubles; includes native file source/drop policy and GNOME/KDE clipboard cases. |
| JavaScript unit runner | 74 passed | Selector, snapshot and text-size tests. |
| New file-drag browser contract | 30 checks passed | Shared UI in Chromium, fictional data and simulated native bridge. |
| Existing release / rc4 / type-to-select browser suites | 68 / 29 / 61 passed | Shared UI with simulated filesystem/desktop data. |
| Standalone website suite | 103 passed | Generated shared HTML, documentation links, sample preview and keyboard behavior in Chromium. |
| Website TypeScript check | Passed | `pnpm check`, dependency-aware `tsc --noEmit`. |
| Standalone design regeneration | Passed | `pnpm designs`; this step is not a Next.js build. |
| Native file transport and Chromium receiver | 27 passed | Real GTK3/WebKit source, GTK URI receiver, separate headed Chromium with readable DataTransfer.Files, internal copy proposal, Escape and tab transport coexistence on Xvfb/X11. Synthetic HTML/controllers and disposable local files; not the full application or installed T3 Code. |
| Existing native tab transport | 24 passed | Production class through ctypes GTK3 adapter, DrawingAreas, isolated Xvfb/X11 pointer gestures. |
| Next.js production build | Passed | `pnpm build`; compiled, typechecked and generated all 19 static pages. |
| Package structure/metadata verification | 52 passed | Package built without installing it; native modules and bridge shipped, checksums, licenses, permissions and syntax verified. |

The first design/check attempts identified missing local dependencies and a TSX
narrowing error. Dependencies were installed with pnpm, the shared TSX fallback
was corrected, and both checks then passed. A real GTK probe also confirmed the
receiver replaces WebKit's pre-existing URI target ID instead of appending a
duplicate. The new clipboard tests use queued selection doubles; actual GNOME
and KDE clipboard owners were not exercised.

A first tab-transport run overlapped another pointer test and failed to start;
the isolated rerun passed all 24 checks. A same-process WebKit receiver returned
an empty browser file list, so the external-file test uses a separate Chromium
process. This confirms native local-file interoperability with Chromium, not
every WebKit receiver or receiving application.

Known limits: installed T3 Code, real SMB mounts, Wayland drag/portal behavior,
and sandboxed receiving apps require testing on the target desktop. File drag
sources offer copy only. ZIP members require extraction; editors requiring local
files need existing local network mount paths. No source deletion, automatic
mounting, browser profile changes or file-manager default changes are made.

The gap analysis is in [FILE-INTERACTION-GAPS.md](../docs/FILE-INTERACTION-GAPS.md).

---

# OpenXplorer 1.0.0-rc.4 — executed test report

Date: 2026-09-07. Debian version: `1.0.0~rc4`.

This is a focused mouse/tab-navigation release. It adds middle-click opening
and fixes the tear-out regression while retaining native tab merging. It does
not claim a fix for every Software center's pre-install local-package view.

## Results from this release's source

| Check | Result | Scope |
|---|---|---|
| Python regression suite | 487 passed | Production logic, disposable local files and explicit desktop-library doubles; 27 new tear-out policy checks. |
| JavaScript unit runner | 74 passed | Selector, snapshot, text-size and helper tests. |
| Existing application browser suites | 345 passed | Actual application HTML in Chromium with simulated file/desktop transport. |
| Middle-click browser checks | 29 passed | Real middle-button gestures, background/foreground tabs, sidebar, grid, SMB target URIs, cached results and mouseup-only compatibility. |
| UI-to-Python ZIP contract | 11 passed | Actual dispatcher and extractor, temporary local adapter; graphical launching intercepted. |
| Two-window UI-to-Python contract | 28 passed | Actual dispatcher/transfer broker and isolated default-association services with native-mode Chromium pages; not GTK/WebKit. |
| Native GTK drag transport | 24 passed | Real GTK3 windows, production drag class through a ctypes adapter, Xvfb/X11 and XTest pointer gestures. |
| Standalone website | 103 passed | Generated shared HTML, simulated sandboxed app, Markdown copying and relative links; not a Next.js production build. |
| Mobile/docs/date checks | 162 passed | Chromium narrow viewports, mobile navigation, no mobile iframe, readable snapshot dates; not physical Android. |
| Website syntax transpilation | 13 files passed | Syntax only, not dependency-aware typechecking or hydration. |
| Package verification | 49 passed | Installed bytes, native action registration, metadata, icons, AGPL notices, permissions, dependencies and syntax; no installation. |
| Project-specific security checker | 20 passed | Source/configuration invariants; not an independent security audit or dependency-vulnerability scan. |
| Native GIO integration | 4 skipped | PyGObject/GIO unavailable. These are not passes. |

The existing browser subtotal is 68 + 61 + 65 + 53 + 27 + 52 + 19.
Including the new middle-click suite, application-browser checks total **374**.
Logs and machine-readable results are in `desktop/test-results/` and
`test-results/` in the complete bundle. Source ZIPs omit generated evidence,
not the scripts needed to reproduce it.

## What the native drag test actually demonstrates

The test uses real native GTK3 top-level windows, with DrawingAreas substituting
for WebKit views. It covers merging, blank-strip drops, reordering, source-body
tear-out, no-target handling, Escape, rejecting another window's body and
merging back after a tear-out. A separate GTK test receiver advertises the
`application/x-rootwindow-drop` target and verifies that the real selection
exchange completes with an empty payload and one deferred new-window request.
It is **a protocol test receiver, not Mutter or a real Wayland desktop**.

The private merge target remains SAME_APP, and the desktop target supplies no
path, password or transfer capability. Source removal still requires the
existing destination-ready acknowledgement. Generic errors and cancellations
are not treated as evidence that a new window should be created.

The transport harness receives the new-window request; it does not start a
complete WebKit window. The existing two-window browser/backend contract tests
the separate handoff and acknowledgement code. Those two layers are not a
claim of a complete end-to-end native application run.

## Package and source correspondence

The package verifier checks the delivered application's Python and UI bytes
against this source tree, including the new middle-click handler and the native
desktop-drop target. The release process rebuilds the package from a clean
extraction of the corresponding-source ZIP and compares SHA-256 hashes; the
outcome is recorded in the separate release-verification JSON. No installation,
credential change, desktop-default change or live network mount is part of
these tests.

## Publication data

All product screenshots were recaptured from the actual HTML with fictional
fixtures. The supplied user screenshots are not in the release. The archive
privacy audit checks known private-term fingerprints, nested ZIP/deb contents,
and screenshot hashes/provenance; it does not use OCR or guarantee detection of
unknown identifiers. Matching source/download links are staged in the website.

## Unverified / excluded

- Full application on Zorin/GTK/WebKit or a Wayland compositor, real SMB access,
  graphical terminal startup and keyring behavior.
- A clean Zorin Software pre-install view. Installed metadata is packaged;
  the screenshot supplied by the user shows it is recognized after installation.
  A plain local DEB cannot guarantee a matched Software card before catalog data
  is available. No signed repository has been published or configured here.
- Next.js production build, React hydration, resolved dependency audit and
  website deployment. The environment lacks the installed pnpm dependencies;
  no lockfile or successful production build is fabricated.
- Native menu/runtime harness results from previous candidates are not counted
  as new runs here. Their code remains available for target-device checks.

The initial combined website/mobile test command exceeded its execution time
budget partway through mobile checks. The website suite completed successfully;
the mobile suite was then rerun independently and all 162 checks passed. There
are no hidden failed assertions counted as passes.
