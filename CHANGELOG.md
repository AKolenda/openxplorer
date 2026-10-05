# Unreleased

- Group by, as in Windows Explorer: Sort > Group by groups a folder apart
  from its sort, so a folder grouped by date modified can be sorted by name
  within each group. The choices are Name (A - H, I - P, Q - Z), Date
  modified (Today, Yesterday, ... A long time ago), Type, Size, Date
  created, Same as sort (the former Show in groups) and (None). Downloads
  is grouped by date modified until another choice is made there. The Sort
  menu now matches Explorer's: Name, Date modified and Type, with Size and
  the further keys under More. The first group's heading is no longer
  hidden when a grouped folder opens.
- No more crash when Details columns change while groups are shown, such as
  going Back from the Recycle Bin to Downloads grouped by date.
- Group headings keep their counts right ("Today (3)") as files are added
  or removed, and date groups move on at midnight: today's files become
  Yesterday's without opening the folder again.
- Turning groups off in a long Details list no longer crashes with GTK 4.22
  ("gtk_list_item_manager_ensure_items"), so (None) is saved and the groups
  go away. The headings now come off before the list changes and go back on
  after, whenever the grouping changes.
- A grouped list at its top stays at its top, with the first heading in
  sight, when it changes: a file dialog opening on Downloads or showing
  another file type, another sort or grouping, files added or removed.
  GTK kept the row that was at the top edge there, which scrolled "Today"
  out of sight or cut it in half.
- Choosing an item in a side menu, such as Sort > Group by > Date
  modified, closes every menu, and a click anywhere in the window closes
  any menu still open. On KDE Plasma the Sort menu could stay on screen
  until a window of another app was clicked.

# 2.0.2 — 2026-10-04

More of Windows 11 File Explorer's behaviour: ZIPs, the Open and Save
dialogs, Delete, Compact view and the navigation pane.

- Extract all works like Windows Explorer's: it appears in the command bar
  while a ZIP is selected, and its dialog is as short as Explorer's, with
  one field, "Files will be extracted to this folder", filled in with the
  ZIP's folder and name, and Browse…. A missing folder is created; an
  existing one (such as Downloads) receives the files directly, asking
  before any file is replaced; a ZIP holding one folder of the same name is
  no longer nested (`tidewater/tidewater`). The counts, notes and Open in
  archive manager are behind the (i) button; a password-protected ZIP says
  so straight away and offers the archive manager, which no longer reopens
  the archive in OpenXplorer when OpenXplorer is the default for ZIPs.
- ZIPs can open like folders, as in Windows Explorer: Settings > Windows &
  tabs > Open ZIP files > Like a folder (Windows). The ZIP opens in the tab,
  with the address bar, crumbs, Back and Up working through it, Extract all
  in the bar, and Copy, Paste and dragging items out (as real copies). It
  stays read-only. The default, In a pop-up window, keeps today's window.
- Copies made from inside a ZIP are cleaned up: a file opened from a ZIP no
  longer leaves its copy in memory until logout (it is removed ten minutes
  after it was opened), and items copied or dragged out are removed after a
  day, when OpenXplorer starts or copies again.
- Open and Save dialogs behave more like Windows':
  - Escape closes the dialog while a file is selected in the list,
    instead of only clearing the selection.
  - The keyboard starts in File name with the name selected (without its
    extension), so typing replaces it instead of jumping through the
    file list.
  - Open dialogs have a File name box too; File name takes a path from the
    folder shown, `~/...` or a full path, and a folder typed there opens.
  - Save adds the chosen type's extension to a name without one, Chrome's
    types included; a name that ends in a dot is saved without one.
  - A file typed in the address bar is chosen instead of being opened in
    another application; in a Save dialog, that file or one double-clicked
    is replaced after asking.
  - Alt+Left, Alt+Right and Alt+Up work from the File name box.
  - Ctrl+Q cancels the dialog instead of closing every window. Ctrl+N,
    Open file location in new tab or new window, and Split view open
    nothing from it.
  - A dialog for one file keeps one item selected.
  - Several files selected in an Open dialog for several are all chosen:
    File name lists them in quotes (`"a.txt" "b.txt"`), and a quoted list
    typed there opens every file in it.
- Holding Shift while clicking Delete, in the command bar, the right-click
  menu or the folder tree's menu, deletes permanently after asking, as in
  Windows Explorer. Before, only the Shift+Delete key did.
- The note at the bottom of the Details pane ("Select an item to see its
  properties…") uses the pane's whole width instead of a narrow column.
- Compact view, as in Windows 11: View > Compact view, and the same switch
  in Settings > Appearance > Files and folders, draws the Details rows and
  the navigation pane's rows closer together, so more items fit. Off by
  default. (Not the List layout, which Dolphin calls "Compact".)
- The arrows beside This PC and Network in the sidebar now collapse and
  expand those sections, as in Windows, with their own highlight; Left
  and Right on the section's row do the same from the keyboard. Clicking
  the name still opens the place. While the open place is inside a
  collapsed section, the section's row is highlighted.
- Settings > Appearance > Layout > "Hide expand arrows", as in Windows:
  the navigation pane's arrows (This PC, Network and the folder tree)
  show only while the pointer is over the pane or keyboard focus is in
  it. The arrows show by default. The file list's folder arrows stay
  with Settings > Appearance > Files and folders > "Expandable folders".

# 2.0.1 — 2026-10-02

Most of the gaps left by 2.0.0 are closed: 635 of the tracked behaviours of
Dolphin, Windows 11 File Explorer and the 1.x app are now native. The 67 still
open are listed in [native/BACKLOG.md](native/BACKLOG.md).

- **Views:** a Compact view, Dolphin's per-folder view properties ("Remember
  display style for each folder"), Show in groups (headed groups in Details),
  more sort keys, configurable columns, zoom with Ctrl+wheel and thumbnail
  previews from the desktop's thumbnail cache.
- **Split panes** (F3), each with its own tabs, history and selection, and a
  folder tree that expands folders in place in Details.
- **Sessions:** with Settings > Windows & tabs > "Restore previous tabs at
  startup" on, the tabs, split panes and histories come back at the next
  start.
- **Tabs:** tab numbers, Close other tabs, reopen closed tabs
  (Ctrl+Shift+T), open several folders in tabs, and closing or quitting is
  guarded while files are written.
- **File operations:** Undo and Redo for copy, move, rename, new items,
  duplicates, links and Recycle Bin; batch rename; New link; a richer name
  conflict dialog; per-file progress; independent transfer jobs with speed,
  remaining time and their own Cancel; a report of what an interrupted copy
  left behind.
- **Navigation and commands:** breadcrumb subfolder menus, Back/Forward
  history menus, typed-address history and completion, Recent locations,
  template menus for New, offline help and a shortcuts list, opt-in service
  actions and Open as administrator (asks first, through GVfs and polkit).
- **Network:** SFTP, FTP, WebDAV and NFS besides SMB, with servers found on
  the network; Disconnect for remote mounts; the Sharing tab in Properties.
- **Search:** file contents, wildcards, live search of folders without an
  index, saved searches, and Kind and Date filters.
- **Selection and keyboard:** rubber-band selection, type-ahead, F6/F8 focus
  cycling and screen-reader names and announcements.
- **Look:** accent colours from Zorin and the desktop portal, desktop text
  scaling, emblems.
- **Optional: other applications' Open and Save dialogs in OpenXplorer.**
  Settings > Default apps > "Apps' Open and Save dialogs" > Enable makes
  applications that use the desktop portal (Chrome, Firefox, Flatpak apps)
  choose and save files in an OpenXplorer window. It is off until enabled,
  keeps every other portal backend, and Restore Open and Save dialogs undoes
  it. The Flatpak cannot offer it. On KDE Plasma, Enable also adds a login
  script, `~/.config/plasma-workspace/env/openxplorer-file-dialogs.sh`, that
  sets `PLASMA_INTEGRATION_USE_PORTAL=1`, so KDE's own apps (Plasma and its
  widgets, Kate, System Settings) follow from the next login; Restore
  removes it, and a file of the user's with that name is left alone.
- **Flatpak:** Show in folder works inside the sandbox and the System theme
  follows a dark desktop.
- **Translations:** the interface uses message catalogues; no reviewed
  language ships yet.
- The Python app of 1.x is no longer in the source tree; its last release is
  1.1.4 and its final sources are `desktop/` at tag v2.0.0.

## Known gaps

The open items, the owner decisions still pending and the hardware acceptance
still owed (SMB servers other than the maintainer's, phones, USB drives,
Orca, mixed-DPI Wayland) are listed in [native/BACKLOG.md](native/BACKLOG.md).

Rollback: `sudo apt install --allow-downgrades ./openxplorer_2.0.0_all.deb`
returns to 2.0.0; settings and saved passwords are shared.

# 2.0.0 — 2026-09-28

OpenXplorer is now a native GTK 4 application written in Rust. It replaces the
Python/WebKitGTK app of 1.x under the same name, command, application ID
(`io.winspace.Development`), settings, pins, search cache and saved SMB
passwords. OpenXplorer 1.1.x offers it in **Check for updates**.

- The same Windows 11 File Explorer look, now drawn with native GTK 4 widgets
  instead of a web view.
- Microsoft Fluent icons for folders, files, drives and commands.
- A categorised Settings page: a category list on the left, one category at
  a time on the right.
- Undo and redo (Ctrl+Z, Ctrl+Shift+Z) for copy, move, rename, new files and
  folders, and Move to Recycle Bin.
- Drop files onto a program or script to run it with those files.
- Tear a tab out into a new window, or drag it onto another window to merge
  it.
- Pinned folders are indexed for search by default.
- Packages for more distributions: the `.deb` for Zorin OS 18, Ubuntu 24.04
  and newer and Debian 13; RPMs for Fedora and openSUSE and a package for Arch
  Linux where the release lists them; and a Flatpak bundle for every
  distribution with Flatpak, including those whose GTK is older than 4.14.
- The Python app in `desktop/` is deprecated and no longer shipped (its sources remain at tag v2.0.0).

## Known gaps

- The **Location** tab of a known folder's Properties (moving Documents,
  Downloads and the other user folders) is a placeholder.
- The mount assistant's **Location** section (choosing where a persistent
  share is mounted) is not available yet.
- A few network protocols that Dolphin offers beyond SMB are not supported.
- Not yet in the native app: searching within Settings, dropping onto
  `.desktop` launchers, creating links and batch rename.
- Behaviour that is implemented but not yet covered by an automated native
  test, and the hardware acceptance still owed (SMB servers other than the
  maintainer's, phones, USB drives), are listed in
  [native/BACKLOG.md](native/BACKLOG.md).
- Inside the Flatpak, "Show in folder" cannot be turned on, the System theme
  stays light on a dark desktop, and settings are kept apart from a
  distribution package's (native/packaging/README.md, "Differences inside the
  Flatpak").

Rollback: `sudo apt install --allow-downgrades ./openxplorer_1.1.4_all.deb`
restores the Python app; settings and saved passwords are shared.

Earlier releases: [desktop/CHANGELOG.md at tag v1.1.4](https://github.com/AKolenda/openxplorer/blob/v1.1.4/desktop/CHANGELOG.md).
