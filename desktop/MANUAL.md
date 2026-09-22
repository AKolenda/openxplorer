# OpenXplorer 1.1.3

Explorer-style file manager for Zorin, with a Python GTK3/WebKitGTK host and a
local HTML/CSS/JavaScript interface. GIO/GVfs provides filesystem, SMB and
connected-device access. This is a maintenance release, not a
production-certified Explorer replacement.

The source archive contains all application source, the interface, tests,
packaging/verification tools, this documentation, an offline preview, and the
matching package in `dist/`. No project files must be downloaded individually.
Distribution libraries are **not vendored**; APT installs those dependencies.

## Retained from 0.7.0

Connected/browsed SMB locations now appear under Network without first being
mapped as saved bookmarks. Tabs can move into separate native windows with an
acknowledged handoff. One Gtk.Application manages the windows and supplies stable
launcher actions for New window, Open windows and Settings.

Settings has keyword search with highlighted matching controls and a fixed,
full-width custom-cache-path input. Default-file-manager setup adds opt-in
FileManager1 support and a Zorin/Brave guide. Downloads relocation can explicitly
sync selected native Brave profiles after consent and after the browser is closed.
The package includes AppStream name, icon, license and release metadata.

**Read ZORIN-SETUP.md** for exact setup steps, portal limitations, taskbar actions,
Brave backup/restore and uninstall safety. No website is required for the installed
folder icon; Software’s local-package preview can still use a generic cached card.

## Install or upgrade

Close **all** running OpenXplorer windows, including secondary windows, first. From 0.7 onward, use `openxplorer --quit`
to stop all windows and the optionally enabled background reveal service safely.
In the folder containing the downloaded package:

```sh
sudo apt update
sudo apt install ./openxplorer_1.1.3_all.deb
openxplorer --check
openxplorer
```

From the extracted source directory, the package is under `dist/` instead:

```sh
sudo apt install ./dist/openxplorer_1.1.3_all.deb
```

Do not launch the graphical application with sudo. The package uses the same
`winspace-explorer` package name as 0.1–0.6.0, so installing it upgrades those
versions. It does not delete your settings or cache, change your default file
manager, redirect Downloads/Documents, create system mounts, or save passwords
at installation time. Normal desktop/icon caches are refreshed.

The intended target is Zorin 18 with the distribution's Python/GI, GTK3,
WebKitGTK 4.1 (2.40 or newer), Secret Service, and GVfs packages. This package is
`Architecture: all` because the shipped application is Python, JS, CSS, and SVG;
APT installs architecture-appropriate native dependencies. Older distributions
may not have the required WebKit version. Do not add unrelated PPAs to install it.

The menu entry is **OpenXplorer**. It uses an unframed yellow
folder icon. Existing pinned dock entries may need unpinning and re-pinning.

### Startup troubleshooting

```sh
openxplorer --check
openxplorer --software-rendering
```

`--check` checks native imports/versions; it is not an SMB connectivity test.
Close existing instances before testing software rendering. It changes only
that launch, not your desktop graphics settings. Run from a terminal to see
errors. Include that output when diagnosing a failure; do not include passwords.

## Features in this source/build

* Light/dark/system appearance, tabs, Home opening the actual home directory,
  editable local/UNC addresses, mouse side-button navigation, and Details.
* Compact Windows 10 context menu by default; switch to Windows 11 in Settings.
  Open with, recognized VS Code/VSCodium installations, Properties, standard
  folder Location settings, New file and real document templates.
* SMB bookmarks, pointer-based sidebar pinning/reordering, a green network icon
  marker, network discovery through available GVfs providers, and server sign-out.
* Connected Android and iPhone locations exposed by GVfs appear under This PC.
  Unlock the phone and select file transfer or trust the computer when prompted.
  OpenXplorer does not mount, probe or index a phone automatically.
* In-app network credentials. Checked Remember requests permanent keyring storage;
  unchecked requests session-only credentials, reusable across shares on the
  same server/port. Successful credentials are reused, not rejected attempts.
* Desktop file clipboard shared across OpenXplorer windows; Ctrl+N opens another
  process/window. Copy/cut/paste still asks about duplicate-name policy.
* MIME-based file opening excludes OpenXplorer's own SMB handler. Activation queries
  current metadata rather than trusting a stale cached folder/file flag.
* Read-only ZIP directory browsing without extracting the complete archive.
  Opening a supported individual member creates one private, read-only temporary
  copy. ZIP contents are never edited in place.
* Local SQLite filename/path cache with local filesystem event updates while
  running; SMB uses incremental directory checks. Mounted drives can be roots.
* Previous versions browses supported **exposed snapshot/backup directories**,
  not Windows SMB shadow-copy enumeration. Restore creates a separate copy.

The installer contains the integrated 0.4 and 0.5 work, not just the old 0.3 UI.

## Retained from 0.6: layout and navigation

* Drag the thin divider between the sidebar and file list. The width is saved.
  Width is bounded by available window space (140–560 px). Double-click resets
  it. The focused separator also responds to Left/Right and Home.
* Drag column-header edges to resize Name, Date modified, Type, Size, and search
  Folder path. Double-click an edge to fit up to 2,000 loaded rows. Widths are
  saved separately by field. Wide layouts scroll horizontally without hiding
  the Type/Size columns. Settings can reset all layout widths.
* Local and SMB addresses use a separate clickable button for every ancestor.
  Long paths scroll rather than becoming one button or losing intermediate parts.
  Ctrl+L, Alt+D, or the address dropdown shows the complete editable path. Click
  a segment to navigate, or middle-click it for a new tab.
* Network-backed tabs carry a green bar below their folder icon, including
  supported CIFS mount paths and snapshot tabs. This denotes location type,
  not connection health. The sidebar retains pins and drops but no longer shows
  the Quick access heading/icon/count.
* Settings opens in its own full-page tab, not a dialog. Appearance/layout,
  filename indexing, and default-file-manager controls remain available. Opening
  Settings again reuses the tab. Ctrl+, opens it directly.
* Properties belongs to the tab that opened it. Browse in Previous versions opens
  a snapshot in another tab; returning to the first restores the same dialog,
  active subtab and versions list without refetching it. Closing the originating
  tab, explicitly closing the dialog, or exiting the app discards that dialog.
* Identically named editor launchers are deduplicated; hidden URL helper entries
  are excluded. Distinct names such as Code Insiders/VSCodium remain distinct.
* Keyboard type-to-select no longer outlines the entire pane in blue. Selected
  items and interactive controls retain focused/selected styling.

### On-demand folder sizes

Right-click a directory/share → **Calculate folder size**, or use Properties →
General → Calculate folder size. Multiple selected directories are queued.
Right-click blank list space → **Calculate folder sizes** scans displayed folders.
No scan starts simply because a folder is opened. Progress and cancellation stay
visible in a bottom strip, and a dedicated worker does not occupy browsing workers.

The Size column and Properties show the same measured **logical file bytes**.
File contents are not read or downloaded. The result is kept only for this window
session, has a timestamp/status tooltip, and does not automatically update after
writes. Recalculate when needed. A stopped/incomplete scan is marked as partial
(a lower-bound `≥` value), never presented as a complete zero-byte directory.

Local directories and mounted filesystem paths use local metadata enumeration;
SMB URIs use GIO metadata enumeration. Native SMB scanning is not tested in the
build environment. A whole SMB server is not a folder; select a share first.

Scans are limited to one million entries or five minutes per selected root,
checked between I/O operations. A blocked filesystem call may return later than
that. Links, special files, nested mounts/filesystem boundaries, and snapshot
collections (.zfs, .snapshot, #snapshot, .snapshots) are excluded; hidden ordinary
files are included. An explicitly selected snapshot directory can be scanned.
Hard links are counted once when the provider supplies stable identity (local
provider); SMB totals may count separate hard-link names separately. Unreadable
entries and exclusions are reported as incomplete coverage. A live directory
changing during enumeration is not an atomic point-in-time measurement.

This is **not** a server-side ZFS size query, an allocation/compression measurement,
or snapshot-exclusive block accounting. ZFS dataset `used`, `referenced`, and
`logicalused` are different metrics and can include descendants/metadata/shared
snapshot effects. Reading an ordinary subfolder through SMB does not make it a
separate ZFS dataset. No SSH command, NAS API, or administrator credential is used.

References: OpenZFS zfsprops.7 at
https://openzfs.github.io/openzfs-docs/man/master/7/zfsprops.7.html and GIO filesystem
metadata at https://docs.gtk.org/gio/method.File.query_filesystem_info.html.

## Type-to-select

Click a file/folder row or blank space in the file pane and type `SC`.
The next filename beginning with `sc` is selected and scrolled into view, such
as `scripts`. This does **not** filter the listing, edit the search box, navigate,
query the filename cache, or issue a filesystem/SMB request. It uses names already
loaded in the current view, including entries outside the rendered viewport.

Matching is case-insensitive, uses filename prefixes, and follows display order.
Type characters with less than a one-second gap to refine the prefix. Repeated
single letters cycle through matches with wraparound. After a one-second pause,
typing starts a fresh prefix. A small status-bar hint disappears after the pause.
Backspace edits an active prefix. Escape clears the prefix first; a second Escape
can clear the selection. Enter opens the selected item using the existing opener.
An unmatched prefix leaves selection and scroll position unchanged.

Works in details and large-icon views, local and SMB listings, and already-shown
search results. It does not search subfolders. Search/address/rename/password
inputs, menus, dialogs, modifier shortcuts and provisional IME events are not
interpreted as file-list typing. The buffer is cleared on mouse selection,
navigation, tab/view changes and loss of focus. No setting needs enabling.

## Practical usage

**Search cache:** Settings (gear) → Search & indexing → check a local folder, Local
Disk, mounted volume, or signed-in SMB folder. The first pass enumerates metadata.
Search then uses the database. Results show their actual parent path, and opening
a folder leaves the virtual search and navigates to that folder's real URI.
Only the first 500 matches are displayed, with an indication when truncated.

Local changes use inotify where available. Watches are capped at 8,192 directories;
remaining directories fall back to incremental checks. SMB is not push-updated
in this version. Set the network/fallback interval in Settings. The index owner
runs only while OpenXplorer is open, and reconciles after restart. Initial scans
are capped at one million entries; system/temporary folders, nested mounts,
symlinks, and snapshot trees are excluded from whole-disk crawling. Select each
mounted filesystem separately. Cached names can be stale and reveal private
filenames locally; they are not offline file contents.

**Network sign-in:** enter `\\server\share` or `smb://server/share` in the address
bar. No separate Domain field is shown; `DOMAIN\username` is supported when
necessary. Unchecked Remember means the Linux login session (ends at logout or
reboot), not merely the current share. Reuse is scoped to the same hostname and
port; IP/hostname aliases are deliberately not merged. Keyring-unlock prompts
can still come from the OS. Without a usable Secret Service collection, credentials
are retained only in this application's memory, with a warning. Passwords are
not stored in the JSON settings or search database.

**Sign out:** right-click an SMB sidebar location → Sign out of server. Close
files first: GVfs mounts may also be used by other applications. Pins remain.
A separately configured persistent CIFS system mount is not removed by this
user-session command. Forget credentials and clear cached filenames are separate
controls. Clearing one cache root cannot clear overlapping roots automatically.

**Downloads on a NAS:** first establish a stable CIFS mount, then right-click
Downloads → Properties → Location. Check the existing writable destination,
explicitly confirm, and Apply. Temporary `/run/user/.../gvfs` paths are rejected.
The operation updates the Linux standard-folder setting and backs up the prior
configuration. It does not relocate existing files. Applications with their own
download-directory setting may need that changed separately.

The optional `winspace-mount-share` helper must be run explicitly with sudo from
a terminal after reviewing its plan. It creates systemd mount/automount units and
a **root-only plaintext credential file**, separate from the user keyring. It does
not edit fstab and never runs during package installation. To print a plan without
making changes (substitute your actual share):

```sh
sudo winspace-mount-share --share '//server/share' --plan
```

Read `mount_share.py` and `mount_support.py` before using privileged setup. Real
mounting has not been tested in this release environment. Do not redirect your
only Downloads/Documents folder to an untested or unavailable share.

**Default file manager:** Settings → Default file explorer → Make OpenXplorer default.
Restore previous is available. This changes per-user folder/SMB associations; it
does not replace application file pickers. The optional Show in folder checkbox now
adds per-user FileManager1 activation/autostart. Use the separate status/test
controls; some browser portal routes may still require choosing OpenXplorer. Read
**ZORIN-SETUP.md** for the complete setup and rollback instructions.
Use Restore previous before uninstalling if you enabled this option.

**Templates:** put a genuine blank document in the configured Templates folder,
then New → From template. An empty file with a `.docx`/`.pdf` extension is not a
valid Office/PDF document. Templates are copied, not executed.

## Drag files into other applications

Select files or folders (Ctrl-click or Shift-click for multiple items), then drag
them into an application that accepts native file drops. Escape cancels.
Drop into an OpenXplorer folder or another window to confirm a copy using
**Replace existing** or **Skip duplicates**. Replacement begins after the new
file is staged; same-name folders merge and retain destination-only files.
Quick access drops pin/reorder folders. No file drop asks the source application
to delete anything.

ZIP contents must be extracted first. Network files are exported using an
already-mounted local path where available; applications without SMB URI support
need GVfs/FUSE or CIFS. Dragging does not mount, authenticate or download files.
The browser preview only simulates pins and cannot export desktop files.

## Important limitations

Use disposable files/test shares first. No Windows ACL editor, file-operation
Undo, cross-filesystem cut/move, drag-to-move, native SMB CHANGE_NOTIFY,
Windows shadow-copy RPC enumeration, archive editing, or true offline content
synchronization is claimed. Where a location has no Trash (SMB shares and most
remote backends), Delete asks for an explicit, clearly labelled permanent delete
instead; a Trash move never silently falls back to deletion. Existing destination files are not intentionally overwritten. The
source may contain undiscovered integration and filesystem bugs.

A green shared-folder marker means network-backed, not necessarily connected.
Network discovery is provider-dependent and may miss devices. Applications need
a compatible local/GVfs path or remote-URI support to open SMB files. ZIP previews
reject unsafe names, symlinks, unsupported encryption, large members and excessive
metadata sizes. Recognized snapshot paths are protected within OpenXplorer, not by
system-wide permission changes.

## Build from source

From this directory on Debian/Ubuntu/Zorin:

```sh
python3 tools/build_preview.py
python3 tools/build_deb.py
python3 tools/verify_deb.py dist/openxplorer_1.1.3_all.deb
```

Only Python 3.10+ and `dpkg-deb` are required for packaging. Optional CairoSVG
adds PNG icon fallbacks; without it the SVG is still installed. This shipped
package includes both SVG and PNG icons. There is no npm build, compiler, private
registry, online build step, or missing generated JavaScript. `SOURCE_DATE_EPOCH`
can control package timestamps. The default is the release date.

To run source natively, install the dependencies declared in the package and use
`/usr/bin/python3 winspace.py`. Do not use a virtual environment that hides GI.
To inspect appearance without native dependencies, open `preview.html` in a
browser. Its files, servers, apps and keyring operations are **simulated**.

## Tests

```sh
python3 -m unittest discover -s tests -p 'test_*.py' -v
node --test tests/type_select.test.cjs       # optional test-time Node.js, not a runtime dependency
python3 tests/ui_release.py                 # requires Playwright + Chromium
python3 tests/ui_type_select.py             # keyboard/focus/virtual scrolling regressions
python3 tests/ui_v07.py                     # windows, network entries, settings search, explicit Brave sync
python3 tests/ui_v06.py                     # resizing, breadcrumbs, settings, tab dialogs, size UI
/usr/bin/python3 -m unittest discover -s tests -p 'gio_integration.py' -v
```

Set `CHROMIUM` to your Chromium executable for browser checks. Test dependencies
are in `requirements-dev.txt`; they are not runtime dependencies. The browser
suite supplies in-memory localStorage because file URL navigation is blocked
in the build container. It exercises the actual interface with the demo transport,
not native WebKit or a live NAS. Native GIO tests skip when GI is unavailable.
See **TEST-REPORT.md** and `test-results/` for exactly what ran in this delivery.

## Source map

`winspace.py` owns the Gtk.Application, native windows and their JSON bridges. `ui/` contains the complete
interface. `ui/type-select.js` is a DOM-independent prefix selector; `ui/app.js`
scopes it to the file pane and handles virtual scrolling and selection. `core.py`, `entry_model.py`, `gio_backend.py` and `operations.py` provide
settings, metadata, storage and conservative transfers. Authentication is in
`auth_bridge.py` and `session_credentials.py`; clipboard in `file_clipboard.py`;
opening in `activation.py`, `native_opening.py` and `file_services.py`; ZIPs in
`archives.py`; caching in `search_index.py`, `index_service.py` and `local_watch.py`.
Properties/location/mount/snapshot services are in `folder_locations.py`,
`mount_support.py`, `mount_share.py` and `previous_versions.py`.
`desktop_integration.py` manages default-handler opt-in/restoration.
`filemanager_bus.py` and `reveal_integration.py` add opt-in FileManager1 service
registration/activation. `window_state.py` validates tab transfers and external
file-reveal requests. `network_locations.py` merges network sidebar sources.
`brave_integration.py` performs explicit offline browser preference edits.
`folder_sizes.py` performs bounded metadata-only folder scans; `app_catalog.py`
filters and deduplicates application launchers without executing them.

Runtime configuration is normally `~/.config/winspace/`; metadata cache is
normally `~/.cache/winspace/`, honoring XDG overrides. Neither is bundled in this
archive. Installed application source is under `/opt/winspace/`.

## License

AGPL-3.0-only; see LICENSE and licenses/Winspace-MIT.txt for preserved upstream notices. No proprietary Windows assets, Microsoft fonts, user credentials,
NAS content or distribution native libraries are included.


## ZIP extraction and text size (1.0.0)

Right-click a ZIP → Extract all…; choose a destination and new folder name. Existing names are never replaced. Progress and Cancel appear in the transfer panel. Double-click browsing stays read-only. Sign into SMB source/destination shares first. See [upgrade notes](UPGRADE.md) for limits and failure recovery.

Ctrl + / Ctrl = increases text, Ctrl − decreases it, Ctrl 0 resets. Settings → Appearance & layout → Text size and the View menu offer the same controls. This does not change desktop DPI.
