# OpenXplorer native backlog

OpenXplorer 2.0.0 shipped the native app before the parity gate
`python3 native/parity/check.py --gate replace` passed: on 2026-09-28 the owner
made the remaining items a backlog instead of a release blocker. Parts 1 and 2
closed most of the initial gaps. Part 3 adds shared dialogs, thumbnail previews,
split panes, folder display styles, commands and transfer jobs, the folder tree
and translation support. This file lists what is still open in the current
source. `native/parity/features.toml` stays the source of truth: an item leaves this list when its status there becomes
"done" (implemented, and a native test carries its `parity: ID` marker).

"partial" means the behaviour is largely implemented but a part of it, or its
native test, is missing. "todo" means it is not implemented natively, or has
no native evidence yet. Each item's `native_note` in features.toml says what
is missing.

## Known functional gaps

- **Owner decisions still open.** These items of the replacement gate are
  implemented as Dolphin and Windows 11 behave and stay "partial" until the
  owner confirms the difference from the Python app:
  - SEL-002: a click on blank space clears the selection but keeps GTK's
    range anchor, so Shift+click extends from the last clicked item.
  - SEL-035 (with SEL-036): a lone Space selects the current item and never
    deselects it, instead of starting a type-ahead prefix.
  - CMD-017: Menu and Shift+F10 in a text field open the field's own menu, and
    Ctrl+H, Ctrl+N, Ctrl+T, Ctrl+W, Ctrl+Tab and Alt+Left/Right/Up also act
    from text fields. Typing with an input method (IME) is not verified.
  - XFER-011: a move that cannot be done natively asks "Move by copying?"
    once per operation; Cancel keeps the old refusal.
- **Checks this environment cannot run.** LOOK-028 (fractional scales and a
  window moving between monitors of different scales need a Wayland
  compositor), ACC-003 (nobody has checked the file list with Orca; the icon
  view exposes no item count or position for undrawn tiles) and PERF-008
  (the supported desktops, see below).
- **View coverage.** Ctrl+plus/minus/0 keep their existing text-size bindings;
  icon zoom uses Ctrl+wheel, the slider and named view shortcuts. Media, rating,
  tag and comment sort keys need a metadata provider. Details shows group
  headings; Icons and List currently show grouped order without headings.
  Preview settings offer a fixed large-file limit; folder artwork stays the
  bundled Fluent icon rather than a collage of the folder's contents.
- **Operation history.** Independent transfers have separate progress and
  cancellation panels, but completed and failed jobs are not retained and
  subsequent jobs are not queued (OPS-033).
- **Interface translations.** Menus, dialogs, Settings, backend messages,
  marked template text and package descriptions now use the catalogue
  tooling. No reviewed language catalogue ships; remaining English plural
  fragments, complete string coverage and right-to-left layout still need
  validation (INT-031).
- **Terminal.** View → Terminal opens the configured external terminal in the
  current folder. An embedded VTE terminal is not available across the current
  supported build targets and Flatpak configuration.
- **Archives.** Browsing and extraction support ZIP and compressed TAR. 7z,
  RAR, encrypted ZIP and ZIP64 still need additional codecs or format support.
- **Flatpak:** "Show in folder" answers only while OpenXplorer runs unless the
  desktop's Background portal starts it at login (native/packaging/README.md,
  "Differences inside the Flatpak"). Flathub publication needs the owner.

## Hardware acceptance still owed

The automated checks run on a private display and session bus with local
fixtures. These need the owner, on real hardware under Zorin OS 18 (Wayland,
then X11), with disposable data:

- **SMB** beyond the owner's own server: other NAS vendors and Samba
  versions, Windows shares, guest and domain sign-in, saved and session
  passwords, rejected credentials, disconnect and reconnect during a copy.
- **Phones** (MTP and PTP, Android and iPhone): browsing, copy to and from the
  phone, replace, rename, delete, unplug during a transfer.
- **USB drives and SD cards**: mount, eject, safely remove, copy across
  filesystems (FAT, exFAT, NTFS), disk full, and "Move by copying?" between
  them.
- **Display and accessibility**: fractional scaling (125, 150 and 175%) on
  Wayland, and the file list, tabs and sidebar with Orca.
- The upgrade from 1.1.4 through **Check for updates** on a real Zorin
  installation, and the rollback to 1.1.4.
- The Flatpak bundle on a distribution without GTK 4.14 (Debian 12), and the
  RPM and Arch packages on their distributions.
- The remaining items of docs/RELEASE-CHECKLIST.md, "Native desktop gate".

## Remaining parity items (67)

Generated from `native/parity/features.toml`; the replacement gate is
every behaviour the Python app had. The other items come from the Dolphin
baseline and GNOME integration. Priority is from the inventory.

### Replacement gate (7)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| SEL-002 | partial | must | Clicking blank file-pane space clears the selection and focuses the list |
| SEL-035 | partial | must | Type-ahead: Space is a prefix character; prefix length is bounded |
| CMD-017 | partial | must | Shortcut scope depends on focus |
| XFER-011 | partial | must | Moves are native only, never copy-then-delete |
| LOOK-028 | partial | must | Sharp rendering at every display scale |
| ACC-003 | partial | must | Consistent keyboard focus in the file list |
| PERF-008 | todo | must | Tested on the supported desktops |

### Dolphin and GNOME additions (60)

#### TAB: Tabs and windows (2)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| TAB-046 | partial | could | Optional full path in the window title |
| TAB-051 | partial | could | Ask before closing a window with several tabs |

#### VIEW: Views, columns, sorting and status bar (20)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| VIEW-010 | partial | should | Icon size zoom levels |
| VIEW-012 | partial | could | Zoom slider in the status bar |
| VIEW-016 | todo | could | Sort-order labels depend on the sort key |
| VIEW-017 | partial | could | 'Folders First' and 'Hidden Files Last' toggles |
| VIEW-018 | todo | could | Choice of sorting mode |
| VIEW-019 | partial | should | More sort keys |
| VIEW-022 | partial | should | Group items by the sort key |
| VIEW-032 | partial | could | Automatic or custom column widths, and side padding |
| VIEW-036 | todo | could | Extra details under icon labels |
| VIEW-038 | todo | could | Automatic recursive folder size |
| VIEW-039 | todo | could | Per-mode label and row layout options |
| VIEW-040 | todo | could | View font setting |
| VIEW-041 | partial | could | Item tooltips |
| VIEW-054 | todo | could | Status bar visibility options |
| VIEW-058 | partial | should | Preview settings |
| VIEW-060 | todo | could | Copy or move to the other split pane |
| VIEW-061 | todo | could | Selection mode |
| VIEW-064 | todo | could | Stash split pane |
| VIEW-065 | todo | could | Version-control status and actions |
| VIEW-066 | todo | could | Permissions column format |

#### SEL: Selection and type-ahead (3)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| SEL-013 | todo | could | Checkbox selection |
| SEL-018 | todo | could | Single-click activation option |
| SEL-019 | todo | could | Touch gestures |

#### SIDE: Sidebar (2)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| SIDE-027 | todo | could | 'Search For' places |
| SIDE-030 | todo | could | Bookmarks menu with saved tab sets |

#### CMD: Command bar and menus (10)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| CMD-020 | todo | could | Paste entry describes the clipboard |
| CMD-021 | todo | could | Shift switches Move to Trash to Delete in an open menu |
| CMD-022 | todo | could | 'Copy To' and 'Move To' submenus |
| CMD-023 | todo | could | Choose which context-menu entries appear |
| CMD-025 | todo | could | Send to → Bluetooth device |
| CMD-026 | todo | could | Send files by email as attachments |
| CMD-027 | todo | could | Send files to a paired phone via Zorin Connect |
| CMD-028 | todo | could | Set an image as desktop background |
| CMD-029 | todo | could | Configure keyboard shortcuts |
| CMD-034 | todo | could | Customise the command bar |

#### OPS: File operations and Recycle Bin (4)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| OPS-005 | todo | could | Create a link to a web or remote location |
| OPS-033 | partial | could | Operation history and queue |
| OPS-034 | partial | should | Duplicate in place |
| OPS-044 | todo | could | Trash size limit and automatic cleanup |

#### XFER: Transfer-engine safety (1)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| XFER-027 | todo | could | Resume interrupted copies |

#### CLIP: Clipboard (1)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| CLIP-015 | todo | could | Paste text or an image as a new file |

#### DND: Drag and drop (3)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| DND-022 | todo | could | Dropping web URLs downloads them |
| DND-023 | todo | could | Accept direct-save drops from archive managers |
| DND-024 | todo | could | Dragging ZIP members out |

#### SRCH: Search and indexing (2)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| SRCH-037 | partial | could | Search filters by type, date, rating and tags |
| SRCH-039 | todo | could | Tags, ratings and favourites |

#### NET: Network shares and sign-in (1)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| NET-034 | todo | could | Choose the character set of a remote server |

#### DEV: Drives and phones (1)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| DEV-013 | todo | could | Online-account drives (Google Drive, OneDrive, Nextcloud) |

#### OPEN: Opening items and applications (1)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| OPEN-022 | partial | should | Embedded terminal panel |

#### ARC: Archives (3)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| ARC-022 | partial | should | Browse archives as folders |
| ARC-023 | partial | should | Compress the selection into an archive |
| ARC-024 | partial | should | Extract non-ZIP archives in-app |

#### PROP: Properties, details pane, previous versions and folder sizes (2)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| PROP-012 | partial | could | Quick Look with Space via the GNOME previewer |
| PROP-013 | partial | could | Media and document metadata in Properties and Details |

#### LOOK: Appearance (1)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| LOOK-018 | todo | could | Device-specific icons (USB stick, SD card, optical disc, phone, camera) |

#### INT: Desktop integration and command line (3)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| INT-002 | todo | could | D-Bus-activatable launcher |
| INT-025 | todo | could | GNOME Shell search provider |
| INT-031 | partial | should | Translated interface that follows the desktop language |

## Bridge operations without a native workflow test

`native/parity/bridge.json` still lists 30 operations of the Python app's
bridge whose native replacement has no end-to-end test: bookmark, braveRestore,
braveSync, cacheClear, cacheRefresh, cacheRemove, cacheSet, cacheStatus,
cacheStop, chrome, connect, environment, focusWindow, list, locationApply,
locationCheck, mountPlan, mountVolume, normalise, open, openTerminal, pin,
preferences, quit, search, uiReady, unmount, window, windowMetadata, windows.
