# 1.0.2

- Copy or move directly when no destination names conflict; show Replace/Skip
  only after checking actual destination names, including hidden entries.
- Offer Windows-style **Replace existing** and **Skip duplicates** choices for
  clipboard paste and native file drops.
- Stage copied data completely before replacing an existing file, and merge
  same-name folders while retaining destination-only entries.
- Keep file/folder type conflicts unchanged instead of deleting a directory as
  a side effect of a batch replacement choice.
- Fix copies to MTP/AFC/SMB-backed locations whose GVfs paths do not implement
  Unix `chmod`; permission hardening remains enabled for real local staging.

# 1.0.1

- Show connected Android MTP, camera/PTP and iPhone AFC devices under This PC
  and in the sidebar, including unmounted devices that can be connected on
  demand through the existing native GIO mount dialog.
- Refresh devices after mount, volume and drive state changes, such as plugging
  in, unlocking or changing the phone's USB mode.
- Support browsing and internal copy operations on GVfs device URIs without
  treating their bracketed USB identifiers as malformed web addresses.
- Keep device mounting opt-in and leave search indexing, terminal launching and
  external drag export disabled for transient device locations.

# 1.0.0

First stable release. The application code is the release-candidate code with the
candidate labelling removed: semantic and Debian versions are both `1.0.0`, so
`apt` upgrades an installed `1.0.0~rc4` normally.

- Publish 1.0.0 as a stable AppStream release entry instead of a development one.
- Ship the installer, corresponding-source archive and `SHA256SUMS` together on
  GitHub Releases; the package is still unsigned and not an APT repository.
- Keep every desktop change opt-in: installation does not alter file-manager
  defaults, browser preferences, folder locations or mounts.
- Native Zorin/Wayland, live SMB and installed-app interoperability remain
  environment-specific checks, as documented in docs/RELEASE-CHECKLIST.md.

# 1.0.0-rc.4

- Add native file dragging to compatible external apps with multiple selection,
  GTK URI/text targets and existing local network-mount mapping.
- Accept incoming and cross-window file drops into folders through the existing
  copy confirmation; keep Quick access pin drops and copy-only source semantics.
- Honor KDE cut markers and consume completed external GNOME/KDE cut items.

- Delete on locations without Trash support (SMB shares, most remote backends)
  now prompts for an explicit permanent delete and carries it out recursively,
  instead of offering a Trash move that can only fail per item.
- Middle-click folders, cached directory results, breadcrumbs, mounted drives
  and sidebar/network locations into a background tab; Shift opens foreground.
- Use mouse down/up with duplicate auxiliary-click suppression for WebKit
  compatibility; regular files are not treated as folders by extension.
- Restore tear-out using GTK's empty desktop-drop target and an explicit source
  body drop area, while preserving acknowledged cross-window merging.
- Defer detach until native drag completion. Escape, rejected destinations,
  generic errors and expired handoffs do not destroy the source tab.
- Preserve installed AppStream icon/license metadata and document why a fresh
  local-package preview needs pre-indexed catalog data before installation.

# 1.0.0-rc.3

- Native tab merge, reorder and detach, plus an existing-window picker.
- Acknowledged tab transfer: busy, failed and expired handoffs retain the source.
- Separate optional ZIP handler and rollback from folder/SMB association settings.
- Current FileManager1 owner and clearer Brave/portal diagnostics.
- Retain the user's explicitly enabled reveal service instead of advertising
  automatic replacement by another file manager.

# 1.0.0-rc.2

Safe restart/version handshake, no GTK fallback menu model, official website metadata,
File Explorer launcher keywords, package-to-AppStream mapping, and ZIP short-read/error handling.
See [the detailed update notes](../docs/HOTFIX-RC2.md).

# 0.9.3 — ZIP extraction and text size

- Added Extract all… to both context-menu styles and the ZIP browser, with preflight, streamed progress, cancellation and no-overwrite publishing into a new folder.
- ZIP files now use the application's yellow folder artwork with a zipper; actual directories ending in .zip remain ordinary folders.
- Added Ctrl + / Ctrl =, Ctrl −, Ctrl 0; saved text sizing from 80% to 200%, View controls and a searchable Settings option.
- Kept the 0.9.2 GTK fallback-menubar suppression, native file-opening policy, fictional website fixtures and mobile preview gating.

# 0.9.2

- Disable the native fallback application-menu row on every application window
  before showing it. Keep shell-exported application actions and existing HTML
  tabs, title metadata, client-side decorations and resize behavior.
- Add source-policy tests and a real GTK 3 / virtual X11 regression harness.
- No settings migrations, storage-engine changes or desktop-wide theme edits.

# 0.9.1

- Right-aligned, provenance-labeled snapshot dates.
- Previous version tab badges and read-only banner.
- Shared real UI preview and a preview-only NAS pinning walkthrough.
- Selected Zorin website, screenshot bento features, Markdown copy/download, no Command-K interception.

# OpenXplorer changelog

## 0.8.0 — 2026-09-06

- Rename visible application and Debian package to OpenXplorer/openxplorer.
- Keep legacy settings, credential identifiers, desktop ID and launch aliases for compatibility.
- Distribute the modified project under AGPL-3.0-only with original MIT notices preserved.
- Add a visible License & source action and Settings section.
- Add Next.js/pnpm website source, three HTML design pitches, documentation and release tooling.
- No storage-engine capability or native-runtime certification is implied by this rebrand.

## Earlier development history

# 0.7.0 — 2026-09-06

Connected/visited SMB Network entries; single native application with multiple
windows; guarded tab handoff; existing-window menu and launcher actions; opt-in
FileManager1 registration/status/test; searchable full-page Settings with highlighted
results; corrected cache input sizing; explicit offline Brave profile Downloads
sync with backups and field-level restore; packaged AppStream/icon/license metadata.
Zorin setup/restore instructions included. No native environment certification.

# OpenXplorer 0.6.0 — layout, navigation and folder sizes

Development release, 2026-09-06, built from the delivered complete 0.5.1 source.

- Persisted sidebar/column widths; pointer and keyboard resize handles, auto-fit,
  synchronized horizontal header/content scrolling, Settings reset control.
- Individually clickable local/SMB path ancestors; full editable path retained.
- Network folder markers on tabs, including snapshot tabs and supported CIFS paths.
- Remove the visual Quick access heading/count while preserving sidebar pins.
- Properties dialogs stay with their origin tab; browsing snapshots no longer
  destroys the original list/dialog. Full-page Settings is a separate reusable tab.
- Deduplicate same-name app launchers and exclude hidden/URL helper launchers.
- Remove whole-pane blue focus outline; preserve selected-item/control focus cues.
- Read-only, bounded, cancellable metadata folder-size scans on a dedicated worker;
  report logical bytes, progress, partial coverage and timestamps. No ZFS API or
  privileged server-size integration. Explicit scans only; no scan on navigation.
- Regression tests, actual browser gestures, source/package verification tools.

## OpenXplorer 0.5.1 — type-to-select

Released 2026-09-06. Focused patch on the delivered 0.5.0 source, preserving its
integrated 0.4/0.5 features and native storage/authentication implementations.

- Type a filename prefix in the file pane to select and reveal a matching row.
- Case-insensitive prefix matching, one-second reset and repeated-letter cycling.
- Handles virtualized details and large-icon lists without rendering all rows.
- Small live status-bar feedback; no filtering, cached-search call or navigation.
- Backspace correction, Escape reset, and existing Enter-to-open behavior.
- Clicks on blank file-pane space now correctly focus the list for keyboard use.
- Ignore text inputs, modal/menu contexts, shortcut modifiers and IME preedit.
- Reset stale selection anchors on folder/tab changes and reuse virtual scrolling
  for arrow/Home/End navigation.
- Ship/test the new script in both the offline preview and the native package.
- Version bump only in the Python core; filesystem and credential code unchanged.

See TEST-REPORT.md for exactly what was run and the native validation limits.
