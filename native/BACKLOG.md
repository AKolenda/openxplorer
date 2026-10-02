# OpenXplorer native backlog

OpenXplorer 2.0.0 shipped the native app before the parity gate
`python3 native/parity/check.py --gate replace` passed: on 2026-09-28 the owner
made the remaining items a backlog instead of a release blocker. Parts 1 and 2
of the backlog work (packaging, tabs, file operations, navigation, network and
search; then views, selection, transfers, safety, properties, settings,
integration, look and accessibility) closed most of them. This file lists what
is still open on `native/2.1-integration`. `native/parity/features.toml` stays
the source of truth: an item leaves this list when its status there becomes
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
  - UPD-022: whether the HTML preview of the website stays a separate
    artifact built from `desktop/ui`.
- **Checks this environment cannot run.** LOOK-028 (fractional scales and a
  window moving between monitors of different scales need a Wayland
  compositor), ACC-003 (nobody has checked the file list with Orca; the icon
  view exposes no item count or position for undrawn tiles) and PERF-008
  (the supported desktops, see below).
- **SAFE-010:** SFTP, FTP and WebDAV addresses accept a plain user name, as
  Dolphin, Files and GVfs do; such a name can reach settings.json, tab titles,
  the clipboard and GTK's recent-servers list. Passwords are always refused.
- **Larger Dolphin features not implemented:** thumbnails (VIEW-057), split
  view (VIEW-059), the compact view (VIEW-008), templates in the New menu
  (OPS-003), the Original location and Date deleted columns of the Recycle
  Bin (OPS-040, VIEW-062), a folder tree (SIDE-028), Recent Locations
  (SIDE-026) and a translated interface (INT-031).
- **Flatpak:** "Show in folder" answers only while OpenXplorer runs unless the
  desktop's Background portal starts it at login (native/packaging/README.md,
  "Differences inside the Flatpak"). Flathub publication needs the owner.
- **Open and Save dialogs (INT-032):** the picker window is not yet made
  transient for the calling application's window (the portal's
  `parent_window`, `x11:` or `wayland:`), so on some desktops it can open
  behind the caller; the Flatpak cannot offer the feature. GTK's own
  startup still creates its inhibit-portal proxy synchronously on desktops
  without a GNOME or Xfce session manager; the backend answers from its
  dispatch thread so this no longer stalls the portal, but a D-Bus-activated
  instance's first window waits until the portal is up.
- **Duplication to remove:** two dialog implementations
  (`window/dialog.rs` and the in-window `dialog_layer`).

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

## Remaining parity items (113)

Generated from `python3 native/parity/check.py --gate replace` and
features.toml. The replacement gate is every behaviour the Python app had; the
other items come from the Dolphin baseline and GNOME integration. Priority is
from features.toml.

### Replacement gate (9)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| SEL-002 | partial | must | Clicking blank file-pane space clears the selection and focuses the list |
| SEL-035 | partial | must | Type-ahead: Space is a prefix character; prefix length is bounded |
| CMD-017 | partial | must | Shortcut scope depends on focus |
| XFER-011 | partial | must | Moves are native only, never copy-then-delete |
| LOOK-028 | partial | must | Sharp rendering at every display scale |
| UPD-022 | todo | must | Offline browser preview with simulated data |
| ACC-003 | partial | must | Consistent keyboard focus in the file list |
| PERF-008 | todo | must | Tested on the supported desktops |
| SAFE-010 | partial | must | Credentials never enter addresses, settings or the UI DOM longer than needed |

### Dolphin and GNOME additions (104)

#### TAB: Tabs and windows (7)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| TAB-017 | todo | should | Tabs opened from a folder appear next to the current tab |
| TAB-018 | partial | should | Dropping files onto a tab |
| TAB-026 | todo | should | Modifier keys when opening a folder |
| TAB-046 | todo | could | Optional full path in the window title |
| TAB-051 | todo | could | Ask before closing a window with several tabs |
| TAB-053 | todo | should | Restore the previous session on startup |
| TAB-055 | todo | should | Configurable startup folder |

#### VIEW: Views, columns, sorting and status bar (36)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| VIEW-004 | partial | must | Dates show both date and time |
| VIEW-008 | todo | must | Compact (list) view mode |
| VIEW-010 | todo | should | Icon size zoom levels |
| VIEW-012 | todo | could | Zoom slider in the status bar |
| VIEW-016 | todo | could | Sort-order labels depend on the sort key |
| VIEW-017 | todo | could | 'Folders First' and 'Hidden Files Last' toggles |
| VIEW-018 | todo | could | Choice of sorting mode |
| VIEW-019 | todo | should | More sort keys |
| VIEW-020 | todo | should | View settings are remembered |
| VIEW-021 | todo | should | 'Adjust View Display Style' dialog |
| VIEW-022 | todo | should | Group items by the sort key |
| VIEW-032 | todo | could | Automatic or custom column widths, and side padding |
| VIEW-033 | todo | should | Choose details columns from the header |
| VIEW-034 | todo | should | Reorder details columns by dragging headers |
| VIEW-035 | todo | should | Expand folders in place in the details view |
| VIEW-036 | todo | could | Extra details under icon labels |
| VIEW-037 | todo | should | Folder item count in the Size column |
| VIEW-038 | todo | could | Automatic recursive folder size |
| VIEW-039 | todo | could | Per-mode label and row layout options |
| VIEW-040 | todo | could | View font setting |
| VIEW-041 | partial | could | Item tooltips |
| VIEW-049 | todo | should | Loading progress and Stop |
| VIEW-051 | partial | must | Status bar shows the total size of items and selection |
| VIEW-052 | todo | should | Status bar describes the selected or hovered item |
| VIEW-053 | partial | should | Free space in the status bar |
| VIEW-054 | todo | could | Status bar visibility options |
| VIEW-056 | todo | should | Monitoring health and stale-state feedback |
| VIEW-057 | todo | must | Thumbnail previews |
| VIEW-058 | todo | should | Preview settings |
| VIEW-059 | todo | must | Split view |
| VIEW-060 | todo | could | Copy or move to the other split pane |
| VIEW-061 | todo | could | Selection mode |
| VIEW-062 | todo | should | Trash view shows Original location and Date deleted columns |
| VIEW-064 | todo | could | Stash split pane |
| VIEW-065 | todo | could | Version-control status and actions |
| VIEW-066 | todo | could | Permissions column format |

#### SEL: Selection and type-ahead (5)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| SEL-012 | partial | must | Rubber-band selection |
| SEL-013 | todo | could | Checkbox selection |
| SEL-014 | todo | should | Hover selection marker |
| SEL-018 | todo | could | Single-click activation option |
| SEL-019 | todo | could | Touch gestures |

#### SIDE: Sidebar (4)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| SIDE-026 | partial | must | Recent files and locations |
| SIDE-027 | todo | could | 'Search For' places |
| SIDE-028 | todo | should | Folder tree panel |
| SIDE-030 | todo | could | Bookmarks menu with saved tab sets |

#### CMD: Command bar and menus (17)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| CMD-012 | todo | should | Background menu offers Sort by, View and Open with for the folder |
| CMD-019 | todo | should | Paste into a selected folder |
| CMD-020 | todo | could | Paste entry describes the clipboard |
| CMD-021 | todo | could | Shift switches Move to Trash to Delete in an open menu |
| CMD-022 | todo | could | 'Copy To' and 'Move To' submenus |
| CMD-023 | todo | could | Choose which context-menu entries appear |
| CMD-024 | todo | should | Service menus and extensions |
| CMD-025 | todo | could | Send to → Bluetooth device |
| CMD-026 | todo | could | Send files by email as attachments |
| CMD-027 | todo | could | Send files to a paired phone via Zorin Connect |
| CMD-028 | todo | could | Set an image as desktop background |
| CMD-029 | todo | could | Configure keyboard shortcuts |
| CMD-030 | todo | should | Show Target of a symbolic link |
| CMD-032 | todo | should | Keyboard shortcuts window |
| CMD-033 | todo | should | Help: offline manual (F1) and issue reporting |
| CMD-034 | todo | could | Customise the command bar |
| CMD-035 | todo | should | Letter shortcuts work on non-Latin keyboard layouts |

#### OPS: File operations and Recycle Bin (13)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| OPS-003 | todo | must | Templates listed directly in the New menu, with subfolders as submenus |
| OPS-005 | todo | could | Create a link to a web or remote location |
| OPS-007 | partial | should | Live warnings while typing a new name |
| OPS-011 | todo | should | Slow second click on a name starts rename |
| OPS-021 | todo | should | Progress with speed and time remaining |
| OPS-025 | todo | should | Several operations at once without blocking browsing |
| OPS-033 | todo | could | Operation history and queue |
| OPS-034 | partial | should | Duplicate in place |
| OPS-039 | todo | should | Administrator access for protected locations |
| OPS-040 | partial | must | Browse the Trash |
| OPS-042 | partial | must | Empty the Trash |
| OPS-044 | todo | could | Trash size limit and automatic cleanup |
| OPS-047 | todo | should | Retry, Skip or Skip all when one item fails |

#### XFER: Transfer-engine safety (1)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| XFER-027 | todo | could | Resume interrupted copies |

#### CLIP: Clipboard (2)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| CLIP-014 | todo | should | Copy path for several items |
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
| OPEN-022 | todo | should | Embedded terminal panel |

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

#### INT: Desktop integration and command line (4)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| INT-002 | todo | could | D-Bus-activatable launcher |
| INT-005 | todo | should | --split starts with a split view |
| INT-025 | todo | could | GNOME Shell search provider |
| INT-031 | todo | should | Translated interface that follows the desktop language |

#### SAFE: Security (1)

| ID | Status | Priority | Behaviour |
|---|---|---|---|
| SAFE-022 | todo | should | Respect the desktop's recent-files privacy settings |

## Bridge operations without a native workflow test

`native/parity/bridge.json` still lists 30 operations of the Python app's
bridge whose native replacement has no end-to-end test: bookmark, braveRestore,
braveSync, cacheClear, cacheRefresh, cacheRemove, cacheSet, cacheStatus,
cacheStop, chrome, connect, environment, focusWindow, list, locationApply,
locationCheck, mountPlan, mountVolume, normalise, open, openTerminal, pin,
preferences, quit, search, uiReady, unmount, window, windowMetadata, windows.
