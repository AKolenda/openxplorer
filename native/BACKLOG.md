# OpenXplorer native backlog after 2.0.0

OpenXplorer 2.0.0 shipped the native app before the parity gate
`python3 native/parity/check.py --gate replace` passed: on 2026-09-28 the owner
made the remaining items a backlog instead of a release blocker. This file
lists them as of the release commit. `native/parity/features.toml` stays the
source of truth: an item leaves this list when its status there becomes
"done" (implemented, and a native test carries its `parity: ID` marker).

Most items are "partial": the behaviour is largely implemented but a part of
it, or its native test, is missing. "todo" items are not implemented natively,
or have no native evidence yet. Each item's `native_note` in features.toml says
what is missing.

## Known functional gaps

- **Properties, Location tab** (folder relocation of Documents, Downloads and
  the other known folders): a placeholder in the native app. Related parity
  items: PROP and INT entries below; partial work is on `native/gaps-props`.
- **Mount assistant, Location section** (NET-027, NET-028): choosing and
  checking where a persistent share is mounted is not ported. The packages
  still ship the Python helper `openxplorer-mount-share` from
  `desktop/mount_share.py`; porting its command line would let the packages
  drop Python entirely.
- **Network protocols only Dolphin offers** beyond SMB (for example SFTP,
  FTP, WebDAV and NFS through GVfs): not offered by the native Connect dialog.
- **Not in the native app yet:** searching within Settings (SET-004), dropping
  onto `.desktop` launchers (DND-020), creating links and batch rename (OPS-014,
  OPS-029), the Undo button in the completion toast (OPS-032).
- **Flatpak:** "Show in folder" cannot be turned on, and the System theme
  stays light on a dark desktop (native/packaging/README.md, "Differences
  inside the Flatpak"). Flathub publication needs the application ID's domain,
  metainfo screenshots and permission exceptions.
- **Cross-track hookups** left open when the tracks were merged
  in `native/rc`: pausing, clearing and resuming a
  server's search index on sign-out and sign-in (NET-022, SRCH-040); telling
  the search index which folders a file operation changed (SRCH-033);
  Properties, cache and Terminal items in the drive and network menus;
  refusing to close a window while a file operation runs.
- **Duplication to remove:** three dialog implementations
  (`window/dialog.rs`, `dialogs/network_form.rs`, the in-window
  `dialog_layer`) and two progress panels (`TransferPanel`, `OperationPanel`).

## Saved gap branches

Partial work toward the items below, not merged into 2.0.0. Rebase each onto
`main` after the release, finish it, and move its items to "done":

| Branch | Area |
|---|---|
| `native/gaps-net` | Network shares, sign-in and the mount assistant (NET) |
| `native/gaps-props` | Properties, including the Location tab (PROP) |
| `native/gaps-nav` | Navigation and address bar (NAV) |
| `native/gaps-tabs` | Tabs and windows (TAB) |
| `native/gaps-view` | Views, columns, sorting and status bar (VIEW) |

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
  filesystems (FAT, exFAT, NTFS), disk full.
- The upgrade from 1.1.4 through **Check for updates** on a real Zorin
  installation, and the rollback to 1.1.4.
- The Flatpak bundle on a distribution without GTK 4.14 (Debian 12), and the
  RPM and Arch packages on their distributions.
- The remaining items of docs/RELEASE-CHECKLIST.md, "Native desktop gate".

## Remaining parity items (306)

Generated from `python3 native/parity/check.py --gate replace` at the
release commit. Priority is from features.toml ("must" is every behaviour the
Python app had).

### NAV: Navigation and address bar (22)

| ID | Status | Behaviour |
|---|---|---|
| NAV-001 | partial | Back and Forward buttons walk per-tab history |
| NAV-002 | todo | Alt+Left / Alt+Right navigate history |
| NAV-003 | todo | Mouse back/forward side buttons navigate history |
| NAV-005 | partial | Per-tab history rules |
| NAV-010 | partial | Up button and Alt+Up go to the parent folder |
| NAV-013 | partial | Refresh button, F5 and Ctrl+R reload the folder |
| NAV-014 | partial | Reloading keeps the surviving selection |
| NAV-015 | partial | Navigation resets transient folder state |
| NAV-016 | partial | Superseded listings are cancelled and ignored |
| NAV-017 | partial | Breadcrumb bar shows one button per ancestor |
| NAV-018 | todo | Breadcrumb buttons: click, keyboard and middle-click |
| NAV-023 | todo | Breadcrumb bar overflow scrolling |
| NAV-025 | todo | Address icon reflects the location kind |
| NAV-026 | todo | Editable address: Ctrl+L, Alt+D, blank-space click or chevron |
| NAV-027 | todo | Cancel address editing |
| NAV-033 | partial | Submitting the address opens the typed location |
| NAV-034 | partial | Typed addresses are normalised: UNC, smb://, local paths, ~ and relative names |
| NAV-035 | partial | Invalid addresses are refused with specific guidance |
| NAV-037 | todo | Opening a file URI as a location opens its parent and the file |
| NAV-038 | todo | Home opens the real home folder |
| NAV-040 | partial | Folder detection uses real metadata, not file names |
| NAV-041 | todo | Externally requested locations open in tabs |

### HOME: This PC and Network pages (3)

| ID | Status | Behaviour |
|---|---|---|
| HOME-001 | todo | This PC page layout |
| HOME-002 | todo | Quick access cards on This PC |
| HOME-011 | partial | Recently opened files (recorded; the Home landing page that shows them is unreachable) |

### TAB: Tabs and windows (28)

| ID | Status | Behaviour |
|---|---|---|
| TAB-001 | todo | New tab via '+' button and Ctrl+T |
| TAB-002 | partial | Closing tabs |
| TAB-003 | todo | A tab that is moving cannot be closed or navigated |
| TAB-004 | todo | Switch tabs by click or Enter |
| TAB-005 | partial | Ctrl+Tab / Ctrl+Shift+Tab cycle tabs |
| TAB-008 | todo | Per-tab scroll is preserved; transient state is cleared on switch |
| TAB-009 | todo | Background tabs load lazily on first activation |
| TAB-010 | partial | Tab titles, icons and tooltips |
| TAB-011 | todo | Previous-version tab badge |
| TAB-012 | partial | Tab context menu |
| TAB-013 | todo | Duplicate tab |
| TAB-019 | todo | Tab strip overflow scrolls horizontally |
| TAB-020 | todo | Middle-click a folder opens it in a background tab |
| TAB-021 | todo | Shift+middle-click opens the folder in a foreground tab |
| TAB-022 | todo | Middle-click on a file does nothing |
| TAB-023 | todo | Middle-click sidebar entries opens background tabs |
| TAB-024 | todo | Middle-click landing-page cards opens background tabs |
| TAB-025 | todo | Middle-click gesture semantics |
| TAB-036 | partial | Tab transfer handshake safety |
| TAB-042 | partial | One process hosts multiple independent windows |
| TAB-043 | todo | Ctrl+N opens a new window at the current folder |
| TAB-044 | partial | Open windows menu from the title bar |
| TAB-045 | todo | Window title follows the active tab |
| TAB-047 | todo | Client-side window caption buttons |
| TAB-048 | todo | Drag the blank title bar to move; double-click to maximize |
| TAB-049 | todo | Guarded window close |
| TAB-050 | partial | Closing a window releases its resources |
| TAB-052 | partial | Quit OpenXplorer closes all windows only when idle |

### VIEW: Views, columns, sorting and status bar (23)

| ID | Status | Behaviour |
|---|---|---|
| VIEW-001 | partial | Details view columns |
| VIEW-002 | partial | Listing metadata is accurate |
| VIEW-003 | partial | Human-readable size format |
| VIEW-005 | partial | Large icons view |
| VIEW-006 | partial | Switching view mode |
| VIEW-007 | todo | View mode persists |
| VIEW-013 | partial | Sort menu |
| VIEW-014 | partial | Sort by clicking column headers |
| VIEW-015 | partial | Sort order rules |
| VIEW-023 | partial | Show hidden files toggle (Ctrl+H) |
| VIEW-024 | partial | Hidden entries are those GIO marks hidden, including .hidden lists |
| VIEW-027 | todo | Details pane toggle |
| VIEW-028 | partial | Resizable columns with saved widths |
| VIEW-029 | todo | Double-click a column edge (or Home) to auto-fit |
| VIEW-030 | todo | Keyboard column resizing |
| VIEW-031 | todo | Name column fills until resized; header scroll follows the list |
| VIEW-042 | partial | Search results show a Folder path column |
| VIEW-043 | partial | Text size shortcuts |
| VIEW-044 | partial | Text size levels and scaling |
| VIEW-045 | partial | Text size is saved and synchronised across windows |
| VIEW-046 | todo | Reset layout widths |
| VIEW-047 | partial | Empty, loading and unavailable states |
| VIEW-055 | partial | The open folder refreshes automatically |

### SEL: Selection and type-ahead (24)

| ID | Status | Behaviour |
|---|---|---|
| SEL-001 | todo | Mouse selection: click, Ctrl+click, Shift+click |
| SEL-002 | todo | Clicking blank file-pane space clears the selection and focuses the list |
| SEL-003 | todo | Right-click selects the item under the pointer unless it is already selected |
| SEL-004 | todo | Ctrl+A selects all displayed items |
| SEL-005 | todo | Escape clears the selection |
| SEL-007 | todo | Arrow Up/Down, Home and End move the selection |
| SEL-008 | todo | Shift+navigation extends the selection from a fixed anchor |
| SEL-015 | partial | Selection lifecycle across navigation, search and reload |
| SEL-020 | partial | Type-ahead: typing a printable character selects the first name starting with it |
| SEL-021 | partial | Type-ahead matching is case-insensitive, Unicode-normalised and prefix-only |
| SEL-022 | partial | Type-ahead: a fresh prefix searches after the current item and wraps |
| SEL-023 | partial | Type-ahead prefix accumulates for 1 second between keys |
| SEL-024 | partial | Type-ahead: repeating one letter cycles through its matches |
| SEL-025 | partial | Type-ahead: an unmatched prefix keeps the selection and reports no match |
| SEL-026 | partial | Type-ahead: Backspace edits the active prefix |
| SEL-027 | todo | Escape clears an active type-ahead prefix before it clears the selection |
| SEL-028 | todo | Type-ahead status-bar hint 'Jump to: <prefix> — <name>' |
| SEL-029 | partial | Type-ahead ignores modified, IME, dead and named keys |
| SEL-030 | todo | Type-ahead is limited to the file-list context |
| SEL-031 | todo | Type-ahead buffer resets on context changes |
| SEL-032 | todo | Type-ahead reveals off-screen matches in virtualised views |
| SEL-033 | partial | Type-ahead follows the displayed order and the visible set |
| SEL-034 | todo | Type-ahead replaces a multi-selection with the single match |
| SEL-035 | partial | Type-ahead: Space is a prefix character; prefix length is bounded |

### SIDE: Sidebar (15)

| ID | Status | Behaviour |
|---|---|---|
| SIDE-001 | partial | Sidebar sections and order |
| SIDE-002 | todo | Clicking a place opens it; clicking the current place reloads it |
| SIDE-003 | todo | Current location highlighted in the sidebar |
| SIDE-005 | partial | Quick access pins |
| SIDE-006 | partial | Standard folders in Quick access follow XDG user dirs |
| SIDE-007 | partial | Pinning folders from menus and panes |
| SIDE-008 | partial | Reorder Quick access pins by dragging |
| SIDE-009 | partial | Unpinning keeps the folder |
| SIDE-014 | partial | Pin context menu |
| SIDE-016 | partial | Drives and devices under This PC |
| SIDE-017 | partial | Drive context menu |
| SIDE-019 | partial | Network locations under Network |
| SIDE-020 | partial | Network location context menu |
| SIDE-022 | partial | Sidebar edits are shared between windows and processes |
| SIDE-023 | partial | Resizable sidebar with saved width |

### CMD: Command bar and menus (10)

| ID | Status | Behaviour |
|---|---|---|
| CMD-005 | todo | Toolbar buttons open their menus below the button |
| CMD-006 | todo | View menu |
| CMD-007 | todo | More options menu |
| CMD-009 | partial | File and folder context menu (classic) |
| CMD-010 | partial | File and folder context menu (Windows 11 compact) |
| CMD-011 | partial | Folder background context menu |
| CMD-015 | partial | Menu interaction model |
| CMD-017 | partial | Shortcut scope depends on focus |
| CMD-018 | todo | Failing commands surface a toast |
| CMD-031 | todo | Clearer menus: app icons and disabled reasons |

### OPS: File operations and Recycle Bin (10)

| ID | Status | Behaviour |
|---|---|---|
| OPS-006 | partial | Name validation messages |
| OPS-020 | partial | Honest, accessible transfer progress |
| OPS-024 | partial | One file operation at a time |
| OPS-026 | partial | Name conflict dialog before copy, move or drop |
| OPS-027 | partial | Conflict check: copy straight away when no names collide |
| OPS-028 | partial | Per-item name-conflict dialog |
| OPS-036 | partial | Paste into a server listing is refused |
| OPS-037 | partial | Errors carry stable codes |
| OPS-038 | todo | Interface crash cancels operations |
| OPS-048 | todo | User templates are bounded regular files, copied privately and never executed |

### XFER: Transfer-engine safety (25)

| ID | Status | Behaviour |
|---|---|---|
| XFER-001 | partial | Copies are staged and published atomically |
| XFER-002 | partial | Only an exclusively created stage is ever cleaned up |
| XFER-003 | partial | Leftover staging is reported with its location |
| XFER-004 | partial | Local staging is private; remote staging is never chmod-ed |
| XFER-005 | partial | Copied folders keep their permissions |
| XFER-006 | partial | Skip policy never touches existing items |
| XFER-007 | partial | A name that appears during a copy is never overwritten |
| XFER-008 | partial | Keep both creates '(copy N)' names |
| XFER-009 | partial | Replace overwrites files and merges folders |
| XFER-010 | partial | Replace falls back to a reversible backup rename |
| XFER-011 | partial | Moves are native only, never copy-then-delete |
| XFER-012 | todo | Moving into the same folder or onto an existing name keeps the source |
| XFER-014 | partial | Trash never falls back to permanent deletion |
| XFER-015 | partial | Permanent delete is explicit, recursive and link-safe |
| XFER-016 | partial | A folder cannot be placed inside itself |
| XFER-017 | partial | Symlinks are copied as links, never followed |
| XFER-018 | partial | Special files are not copied |
| XFER-019 | partial | Batch validation: modes, policies, count, duplicates, roots |
| XFER-020 | partial | Previous-version protection is checked for the whole tree first |
| XFER-021 | partial | Phone (MTP) copies stage beside the final name |
| XFER-022 | partial | Phone staging cleanup retries and verifies absence |
| XFER-023 | partial | Copies within one phone use a private folder |
| XFER-024 | partial | Phone renames and moves |
| XFER-025 | partial | Source folders are relisted after phone moves |
| XFER-026 | todo | Replace on phones never uses the device overwrite |

### DND: Drag and drop (3)

| ID | Status | Behaviour |
|---|---|---|
| DND-006 | partial | Drags cannot start while the window is busy |
| DND-013 | partial | Drops are rejected when the window changed or is busy |
| DND-014 | partial | Drop folders on Quick access to pin them |

### SRCH: Search and indexing (3)

| ID | Status | Behaviour |
|---|---|---|
| SRCH-014 | partial | Opening search results |
| SRCH-020 | partial | Cache toggle in menus |
| SRCH-033 | partial | File operations update the search cache |

### NET: Network shares and sign-in (14)

| ID | Status | Behaviour |
|---|---|---|
| NET-001 | partial | Map network location dialog |
| NET-003 | partial | SMB server listing shows shares as network folders |
| NET-004 | partial | Shares mount automatically when browsed |
| NET-005 | todo | SMB folders without change notifications still refresh manually |
| NET-006 | partial | Network locations include CIFS mount paths |
| NET-009 | partial | Sign-in dialog keyboard and modality |
| NET-014 | partial | Credentials are reused per server without re-prompting |
| NET-016 | partial | Browsing a share does not save it |
| NET-020 | partial | Sign out of server |
| NET-021 | partial | Sign out forgets credentials |
| NET-022 | partial | Sign out can also clear the server's search cache |
| NET-026 | partial | SMB paths resolve to local mount paths |
| NET-027 | partial | Persistent network mount assistant: plan and Location-tab UI |
| NET-028 | partial | Persistent network mount helper (openxplorer-mount-share) |

### DEV: Drives and phones (4)

| ID | Status | Behaviour |
|---|---|---|
| DEV-001 | partial | Live list of drives, volumes and devices from the volume monitor |
| DEV-002 | todo | Hot-plugged drives and phones appear immediately |
| DEV-005 | partial | Phone and camera addresses |
| DEV-006 | partial | Disconnect (unmount) a drive, phone or share |

### OPEN: Opening items and applications (12)

| ID | Status | Behaviour |
|---|---|---|
| OPEN-001 | partial | Activating items: folder, archive or application |
| OPEN-002 | todo | Enter opens the single selected item |
| OPEN-004 | todo | Activation result stays bound to the tab that started it |
| OPEN-005 | partial | Default application chosen by content type, never OpenXplorer |
| OPEN-006 | partial | Network files open through a local path when possible |
| OPEN-007 | partial | Executables never run silently |
| OPEN-012 | partial | Open with launches and can set the default |
| OPEN-015 | partial | Editor shortcuts in context menus |
| OPEN-016 | todo | Open in new tab from menus |
| OPEN-017 | partial | Open in Terminal |
| OPEN-018 | partial | Terminal selection |
| OPEN-020 | partial | Terminal launch semantics |

### ARC: Archives (3)

| ID | Status | Behaviour |
|---|---|---|
| ARC-001 | partial | ZIP files look like compressed folders |
| ARC-005 | partial | ZIP browser size limits |
| ARC-007 | partial | ZIPs on network shares are read in place |

### PROP: Properties, details pane, previous versions and folder sizes (10)

| ID | Status | Behaviour |
|---|---|---|
| PROP-001 | partial | Opening Properties |
| PROP-003 | partial | Properties General tab |
| PROP-008 | partial | Properties belongs to its tab |
| PROP-009 | todo | Details pane contents |
| PROP-017 | todo | Location tab: relocate a standard XDG folder safely |
| PROP-018 | todo | Location tab: Brave follow-up for Downloads |
| PROP-024 | partial | Snapshot and backup locations are read-only |
| PROP-029 | partial | Folder size limits, cancellation and errors |
| PROP-030 | partial | Folder size on network shares |
| PROP-031 | todo | Relocating a standard folder: validation, backup and verification |

### SET: Settings (13)

| ID | Status | Behaviour |
|---|---|---|
| SET-001 | todo | Settings opens as its own tab |
| SET-003 | todo | Settings navigation column |
| SET-004 | todo | Search within Settings |
| SET-005 | todo | Appearance & layout section |
| SET-009 | todo | Folder sizes, Windows & tabs and license sections |
| SET-012 | partial | Settings storage |
| SET-013 | partial | Damaged settings fall back to safe defaults |
| SET-014 | partial | Preference changes from several windows merge |
| SET-015 | partial | Settings apply to every open window |
| SET-016 | partial | Preference defaults and allowed values |
| SET-017 | partial | Older settings files are migrated |
| SET-018 | todo | Standard-folder configuration is parsed, never executed |
| SET-019 | partial | Settings are organised by category, one category at a time |

### LOOK: Appearance (22)

| ID | Status | Behaviour |
|---|---|---|
| LOOK-001 | todo | Windows 11 File Explorer layout |
| LOOK-002 | todo | Windows 11 colour tokens and typography |
| LOOK-003 | partial | Light, Dark and System themes |
| LOOK-004 | partial | Follow the system light/dark style live |
| LOOK-005 | todo | Appearance toggle button and menu |
| LOOK-006 | todo | Native dialogs follow the app theme without changing GNOME |
| LOOK-007 | todo | No white flash while loading |
| LOOK-008 | todo | Startup skeleton in the Explorer layout |
| LOOK-009 | todo | Title bar, tabs and caption buttons geometry |
| LOOK-010 | todo | Explorer-style client-side titlebar |
| LOOK-011 | todo | Window size and application identity |
| LOOK-012 | todo | Navigation bar, address and search box styling |
| LOOK-013 | todo | Command bar styling |
| LOOK-014 | todo | File list and sidebar row styling |
| LOOK-015 | partial | Icon set |
| LOOK-016 | partial | Windows-style network location icon |
| LOOK-019 | todo | Menu styling (classic vs compact) |
| LOOK-020 | todo | Dialog styling |
| LOOK-021 | todo | Toast notifications |
| LOOK-022 | todo | Responsive layout breakpoints |
| LOOK-023 | todo | Previous-version amber styling |
| LOOK-028 | todo | Sharp rendering at every display scale |

### INT: Desktop integration and command line (11)

| ID | Status | Behaviour |
|---|---|---|
| INT-001 | partial | Single-instance application |
| INT-003 | partial | Desktop entry and launcher actions |
| INT-004 | partial | Command-line options |
| INT-006 | partial | --new-window opens locations in a separate window |
| INT-014 | partial | FileManager1 requests: ShowFolders, ShowItems, ShowItemProperties |
| INT-017 | partial | Background service mode for Show in folder |
| INT-018 | todo | Zorin + Brave troubleshooting guide |
| INT-021 | partial | Brave download folder: restore |
| INT-023 | partial | Focus handoff for externally requested windows and launched apps |
| INT-024 | todo | Media insertion opens the new volume in OpenXplorer when it is the default |
| INT-029 | partial | Legacy names and paths kept |

### UPD: Updates, packaging and startup (17)

| ID | Status | Behaviour |
|---|---|---|
| UPD-005 | partial | Installing an update locks the application |
| UPD-006 | partial | Update failure recovery and restart |
| UPD-008 | partial | Launch detects an outdated running process |
| UPD-009 | partial | --restart replaces the running instance safely |
| UPD-010 | partial | --check, --version and --diagnose |
| UPD-011 | todo | Startup failure screen and ready handshake |
| UPD-012 | todo | Startup watchdog with software-rendering recovery |
| UPD-013 | partial | Software rendering option |
| UPD-014 | todo | Missing system libraries are explained |
| UPD-015 | todo | About this build dialog |
| UPD-016 | todo | License & source dialog |
| UPD-017 | partial | Debian package contents |
| UPD-018 | partial | Installation never changes user state |
| UPD-019 | partial | Reproducible package build and verification |
| UPD-020 | partial | AppStream metadata for GNOME Software |
| UPD-021 | todo | Release artifacts and privacy check |
| UPD-022 | todo | Offline browser preview with simulated data |

### ACC: Accessibility (12)

| ID | Status | Behaviour |
|---|---|---|
| ACC-001 | todo | Semantic roles and labels |
| ACC-003 | todo | Consistent keyboard focus in the file list |
| ACC-004 | todo | Dialog focus management |
| ACC-005 | todo | Focus inside dialogs and back to where it was |
| ACC-006 | todo | Everything is keyboard operable |
| ACC-007 | todo | Enter/Space activate focused controls without triggering file-pane Enter |
| ACC-008 | todo | Tab strip keyboard semantics |
| ACC-009 | todo | Visible focus without a pane-wide outline |
| ACC-010 | todo | Reduced motion |
| ACC-011 | todo | Higher contrast |
| ACC-012 | todo | Large text keeps layouts usable |
| ACC-014 | todo | Accessible startup placeholder |

### PERF: Performance (7)

| ID | Status | Behaviour |
|---|---|---|
| PERF-001 | todo | Virtualised file list |
| PERF-002 | partial | Streamed listing batches are coalesced |
| PERF-003 | partial | No file I/O on the interface thread |
| PERF-004 | todo | Filtered and sorted view is cached |
| PERF-005 | partial | Bounded work for search, fit and discovery |
| PERF-006 | partial | Event rates are bounded |
| PERF-008 | todo | Tested on the supported desktops |

### SAFE: Security (15)

| ID | Status | Behaviour |
|---|---|---|
| SAFE-002 | todo | Only explicit actions use the network |
| SAFE-003 | todo | Untrusted strings are rendered as text only |
| SAFE-008 | todo | Refuse to run as root |
| SAFE-009 | partial | Private application state files |
| SAFE-010 | partial | Credentials never enter addresses, settings or the UI DOM longer than needed |
| SAFE-011 | partial | Passwords never reach settings, logs or the page |
| SAFE-012 | partial | Stale credential saves are discarded after sign-out |
| SAFE-013 | partial | Stale asynchronous results are discarded |
| SAFE-014 | todo | Discovery never asks for passwords |
| SAFE-015 | partial | The interface cannot choose what the terminal runs |
| SAFE-016 | todo | Updates use a fixed, allowlisted download chain |
| SAFE-017 | partial | D-Bus reveal requests are data, never commands |
| SAFE-018 | partial | Settings keep only whitelisted, bounded values |
| SAFE-020 | partial | Desktop-integration writes are narrow and reversible |
| SAFE-021 | partial | The mount helper refuses unsafe system paths |

## Bridge operations without a native workflow test

`native/parity/bridge.json` still lists 31 operations of the Python app's
bridge whose native replacement has no end-to-end test: activateItem,
bookmark, braveRestore, braveSync, cacheClear, cacheRefresh, cacheRemove,
cacheSet, cacheStatus, cacheStop, chrome, connect, environment, focusWindow,
list, locationApply, locationCheck, mountPlan, mountVolume, normalise, open,
openTerminal, pin, preferences, quit, search, uiReady, unmount, window,
windowMetadata, windows.
