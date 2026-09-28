# Native rewrite status and next milestones

The native application is a GTK4 preview alongside the Python/WebKit desktop.
It is not ready to replace the installed desktop application. This document is
the plan for getting there. The exhaustive list of behaviours the native app
must provide is [parity/features.toml](parity/features.toml), and
`python3 native/parity/check.py` reports how many are done. Nothing here claims
visual or performance equivalence with the Python app or Dolphin.

## Current milestone: browse folders with the native foundation

The core ports are organised by responsibility:

- [Locations](crates/ox-core/src/location.rs): canonical local, SMB and device
  addresses, validation, virtual places, breadcrumbs and display names.
- [Entries](crates/ox-core/src/entry.rs): GIO metadata, share/shortcut
  classification, worker-thread enumeration, cancellation and pin validation.
- [Settings](crates/ox-core/src/settings.rs): the existing `winspace` JSON schema,
  bounded preferences, pins, recents, atomic replacement and Python-compatible
  `flock` locking. Failed mutations preserve the prior in-memory snapshot.
- [Places](crates/ox-core/src/places.rs): hidden and ordered Quick access,
  network-mount badges and merging saved, mounted and visited SMB locations.
- [Clipboard formats](crates/ox-core/src/clipboard.rs): validated GNOME, KDE and
  existing OpenXplorer payloads, including cut ownership and consumption.
- [Transfers](crates/ox-core/src/transfer.rs) and the
  [GIO adapter](crates/ox-core/src/gio_node.rs): reusable operation machinery,
  separate from GTK confirmations and progress dialogs.

The [GTK components](crates/ox-app/src/) cover Explorer artwork and themes,
folder models, details and icon views, sorting, selection, filtering, typeahead,
text scaling and volume snapshots. The [window](crates/ox-app/src/window/)
assembles these into a browsing preview with per-tab history, navigation,
breadcrumbs, sidebar places and a selection metadata pane. Read-only browsing
is the first application milestone; core mutation support does not imply that
its UI workflow has been implemented or verified.

Tests include synthetic Python/JavaScript location fixtures, real temporary
files, and Python/Rust settings round trips and lock exclusion in both
directions. Those tests establish the behaviours they exercise. They do not
establish native SMB, phone, Wayland drag-and-drop or assistive-technology parity.

## In progress: complete safe file-operation workflows

The window runs the file operations on the core's ops service
([`window/file_ops`](crates/ox-app/src/window/file_ops.rs)), one at a time and
without freezing navigation:

- New folder and the New menu's files, Rename, Duplicate, Move to Trash,
  Shift+Delete, copy, cut and paste, with the Python app's dialogs,
  confirmations and completion reports, the transfer panel with Cancel, and
  Undo and Redo on an application-wide journal.
- The name-conflict dialog (Skip, Keep both, Replace, Apply to all).
- The display clipboard, claimed in all four formats and read asynchronously;
  a read that an owner change overtook is dropped, and a move-paste consumes
  only its own cut.
- The Recycle Bin as a folder, with Restore, Delete permanently and Empty.
- The context menus of files, folders, blank space (both styles), tabs and
  Quick access pins, with the enable rules of the command bar.

Still to do:

- Native file drag-and-drop, cross-window moves, and tabs moved between windows.
- Renaming in place, dimmed cut items, the Recycle Bin's Original location and
  Date deleted columns and its sidebar entry.
- Preserve staging, replacement, cancellation and source-version safety rules
  under real local, remote and removable-device failures.

The local GIO deletion adapter pins directory descriptors so an ancestor
replaced by a symlink cannot redirect recursion. Deleting inside a folder
reached through a symbolic link, and permanently deleting folders on shares and
phones, work again and are tested on local folders and a simulated MTP device.
Real SMB and phone deletion must still be checked by hand before replacing the
existing desktop; passing simulated transfer tests does not close that.

Behavioural sources: [operations.py](../desktop/operations.py),
[file_clipboard.py](../desktop/file_clipboard.py),
[native_file_drag.py](../desktop/native_file_drag.py),
[native_file_drop.py](../desktop/native_file_drop.py),
[native_tab_drag.py](../desktop/native_tab_drag.py),
[tab_transfers.py](../desktop/tab_transfers.py) and the corresponding tests.

## Then: recover the remaining Python application services

Track each service independently; an existing Rust data model is not a completed
service or UI:

| Area | Work still needed before replacement | Existing behaviour |
| --- | --- | --- |
| Network and devices | Mount/auth dialogs, credentials and sign-out, reconnect/errors, live discovery, unmount/eject, phone-specific validation | [auth_bridge.py](../desktop/auth_bridge.py), [session_credentials.py](../desktop/session_credentials.py), [volume_locations.py](../desktop/volume_locations.py), [mount_share.py](../desktop/mount_share.py) |
| Search and metadata | Indexed/cached search, index lifecycle, live changes, folder-size jobs, full properties and open-with flows | [search_index.py](../desktop/search_index.py), [index_service.py](../desktop/index_service.py), [folder_sizes.py](../desktop/folder_sizes.py), [file_services.py](../desktop/file_services.py), [app_catalog.py](../desktop/app_catalog.py) |
| Archives and recovery | Archive creation/extraction, zip-slip protection, cancellation, previous-version browsing and restore, read-only snapshot rules | [archives.py](../desktop/archives.py), [zip_extraction.py](../desktop/zip_extraction.py), [previous_versions.py](../desktop/previous_versions.py) |
| Preferences and sessions | Full settings UI, cross-window preference refresh, known-folder reload/relocation, session/window state and recents presentation | [core.py](../desktop/core.py), [folder_locations.py](../desktop/folder_locations.py), [window_state.py](../desktop/window_state.py) |
| Desktop integration | Explicit default-file-manager setup, FileManager1 reveal/open, external activation, terminal/editor shortcuts, browser integration | [desktop_integration.py](../desktop/desktop_integration.py), [filemanager_bus.py](../desktop/filemanager_bus.py), [activation.py](../desktop/activation.py), [terminal_integration.py](../desktop/terminal_integration.py), [brave_integration.py](../desktop/brave_integration.py) |
| Distribution | Packaging, runtime/dependency diagnostics, update flow, migration and rollback | [runtime_guard.py](../desktop/runtime_guard.py), [updater.py](../desktop/updater.py), [README](README.md) |

The [Python bridge](../desktop/winspace.py) and
[desktop UI](../desktop/ui/app.js) remain the behavioural references. This table
groups the work; [parity/features.toml](parity/features.toml) and
[parity/bridge.json](parity/bridge.json) enumerate every behaviour and bridge
operation, so update their native status as ports land.

## Desktop usability baseline

The requested minimum is Dolphin's functionality, usability and native desktop
integration, while retaining the current OpenXplorer appearance refined toward
Windows 11 File Explorer as specified in [docs/ui-spec.md](docs/ui-spec.md).
The Dolphin baseline is inventoried in [parity/features.toml](parity/features.toml)
(features with origin `dolphin`); `--gate dolphin` passes only when each of its
must-haves is done or does not apply (`n-a`). This checklist records what
acceptance must also compare, not completed comparisons:

- Predictable keyboard navigation, focus restoration, selection, sorting,
  address entry, breadcrumb navigation and tab/window behaviour.
- Responsive large-folder browsing, cancellation, external change refresh and
  clear offline/permission errors; measure startup and interaction latency.
- Consistent details/icon views, thumbnails, scaling, light/dark appearance,
  reduced motion, narrow windows and high-DPI displays.
- Accessible names, focus order and actionable status/error feedback, verified
  with actual assistive technology and keyboard-only use.
- Interoperable clipboard and drag/drop with other desktop applications on
  both Wayland and X11; document backend-specific limitations.
- Implement the gains the inventory lists, such as split panes, batch rename,
  shared system thumbnails and bookmarks, and richer preview and terminal
  integration.
- Compare light/dark, details/icon, menu, sidebar and narrow-window captures
  against the existing OpenXplorer skin. Preserve its layout and interactions;
  refine only as [docs/ui-spec.md](docs/ui-spec.md) specifies, and document any
  other deliberate refinement instead of silently redesigning a surface.

## Replacement and release gates

1. All required Python workflows have an implemented native path, regression
   coverage, and a documented decision for any intentional difference.
2. Run `python3 native/tools/check.py` for inventory consistency, formatting,
   Clippy with the workspace lints, core tests and real GTK integration tests
   in disposable sessions. Python 3 is required by settings interop tests. Require
   `python3 native/parity/check.py --require-replacement --gate replace --gate dolphin`
   before replacement (see [parity/README.md](parity/README.md)), plus the
   manual UI and Dolphin acceptance checks above.
3. Exercise real SMB authentication/reconnect, removable devices, Trash,
   concurrent settings writers, conflict/cancellation/recovery and accessibility.
   Headless GTK and local GIO tests cannot substitute for these checks.
4. Keep desktop integration opt-in. Preserve persisted `winspace` paths, MIME
   contracts, keyring schemas and final application IDs. The preview retains
   its separate application ID until replacement is explicitly authorized.
5. Preserve licensing notices and regenerate the corresponding-source archive
   and website source link from the actual release tree. Run the public-data
   audit after capture generation and again after staging the complete release.
6. Validate packaged installation, update, coexistence and rollback without
   changing a user's defaults, profiles, folders or mounts implicitly.
