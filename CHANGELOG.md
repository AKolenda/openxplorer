# 2.0.16 — 2026-11-09

Updates install without a password for administrators at the computer, search looks and works like Windows 11's, the Preview pane shows what's in a file, and more of Windows 11's File Explorer keyboard shortcuts work. A paste keeps going past an unreadable or missing item, Compact view and group headings match Windows 11, Recent files open as the real file, the Open and Save dialog keeps its size, and a tab's menu opens from the keyboard. Much of this release is the work of @addition-official.

## Security
* In the Debian package, **Check for updates** now installs through a small helper of its own instead of a general administrator command, so an administrator (a member of `sudo`, `admin` or `wheel`) in an active session at the computer installs updates without typing a password. Anyone else, and any remote or inactive session, is still asked for an administrator's password. The helper takes only a version number: it installs only GitHub's current latest official release, only when that is newer than the installed version, and only after checking the download's size, checksum and package details. It never downgrades or reinstalls, runs with a cleared environment, and reads no settings or files the user can change. Trust in the release is the same as before: GitHub's connection and its checksum, not an independent signature.
* It is on by default. **Settings › About › Install updates without asking for a password** turns it off, and turning it back on asks for an administrator's password. Updating to 2.0.16 itself still asks once, since the helper arrives with it. The RPM, Arch, Flatpak and preview packages never update themselves, so they are unchanged.

## What's Changed
* **Preview pane:** View › Preview pane (Alt+P) shows the selected file's contents, as in Windows 11: text and source files as plain wrapped text (the first 1 MB), pictures as large as the pane allows, PDF pages one under another, and audio and video in the pane's player. A folder, several items or a file with no preview says "No preview available.". PDF pages are drawn by `pdftoppm` in the same sandbox and limits as the thumbnailers, and nothing is read for a network file while network previews are off.
* **Search:** searching looks like Windows 11's by default. There is no filter bar above the results: the command bar shows **Search options** (All subfolders or Current folder, Date modified, Kind, Size and File contents) and **Close search**, the tab reads "notes - Search Results in Documents", and the words searched for are highlighted in the names. Size has Windows' groups from Empty to Gigantic, Date modified runs to Last year, and `kind:`, `datemodified:` and `size:` typed in the search box work as in Windows (`kind:picture`, `size:tiny`). "…" › Pin to Quick access saves a search, and results show sooner in a big folder. Settings › Search › Show the search filter bar brings back the filter bar.
* **Keyboard shortcuts:** more of Windows 11's File Explorer keys work: Ctrl+1 to Ctrl+9 show that tab (and that tab of a Properties window), Ctrl+Shift+E opens the navigation pane down to the current folder, the keypad's plus, minus and asterisk open and close folders in the navigation pane, Ctrl+F5 refreshes, and Ctrl+Insert and Shift+Insert copy and paste files. Ctrl+Shift+1 to Ctrl+Shift+8 now work when typed, on any keyboard layout. Alt pressed alone moves the keyboard to the command bar, and Escape gives it back; the command bar's "…" is named See more. The Keyboard shortcuts window finds a shortcut by its keys however they are typed ("ctrl shift e"), and its scroll bar no longer covers the keys.
* **Tabs:** Shift+F10 or the Menu key on a focused tab opens its menu, and Escape gives the keyboard back to the tab. F6 reaches the tab in front, and no outline is left on the tab strip after files are dropped on a tab.
* **Views:** Compact view spaces Details, Tiles, Content and the navigation pane as Windows 11's Compact view does. Group headings are drawn as Windows 11 draws them: the title in regular weight, a quarter larger than the names, with no line after it.
* **Names and sizes:** with Hide extensions for known file types on, every type the desktop knows hides its extension, `sunset.png` and `data.json` included. The Details pane and tooltips write sizes as the status bar does, and the Size column is always wide enough for "9,999,999 KB". Renaming and new items drop periods and spaces at the end of a name, erasing the whole name in place keeps the old one, and names with a line break or that aren't valid UTF-8 can be renamed, copied and recycled.
* **Copying and moving:** a paste no longer stops at the first unreadable folder or missing item: everything else is pasted, and that item is asked about in the progress window (Folder Access Denied, Item Not Found). A folder pasted or dropped into its own subfolder is refused with Windows 11's "Interrupted Action", "The destination folder is a subfolder of the source folder.". Undo after a partial copy or move reverses only what was done.
* **Recent files:** a Recent files row opens, copies, drags and shows Properties as the file it stands for, and says Item Not Found when that file is gone. As in Windows 11's Recent list, it can be copied but not cut, renamed or deleted.
* **Open and Save dialogs:** the dialog opens at the size it was last left at, and on GNOME a dialog whose app's window handle has a space in it now stays attached to that app instead of falling behind it.
* **Details:** dragging a column's edge past its start stops the column at its narrowest width instead of folding it to nothing.

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.16_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.16-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.16-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.16-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.16-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.15...v2.0.16

# 2.0.15 — 2026-11-08

OpenXplorer now looks and works much more like Windows 11's File Explorer, from the command bar, menus and status bar to Properties, This PC and the progress window. It also makes previews with the desktop's own thumbnailers in a sandbox, can paste a picture as a file, and offers to continue as administrator when a copy, move or delete is denied. Much of this release is the work of @addition-official and @clansty.

## Security
* Previews made by the desktop's own thumbnailers, new in this release, run in bubblewrap, as GNOME's Files runs them: with no network, no session bus and no other processes, a private empty home and `/tmp`, and only the previewed item of your files, read-only. Together they may use at most 2 GiB of memory, 64 processes and one processor's worth of time, at a lower priority than the app. Where bubblewrap is missing or can't run, no outside thumbnailer is started.
* A picture whose header asks for more than 64 megapixels (256 for a JPEG), a damaged picture, or one that takes longer than about 10 seconds to decode keeps its type icon instead of being decoded, in thumbnails, the Details pane's preview and custom icons, and is not handed to a thumbnailer either.
* **Continue**, offered when a copy, move or delete is denied, checks the location with the same refusals as the **Open as administrator** folder delete before it asks for a password: a folder other accounts can change, or one on a drive users can mount, is refused and the reason is shown. It uses the existing administrator paths and adds no new privilege mechanism. GVfs has no administrator Recycle Bin, so in a Recycle Bin delete Continue first asks whether to delete the item permanently.

## What's Changed
* **Look:** Windows 11's density, with 32 px rows, 48 px bars and 16 px icons, about 22 rows in a 900 px window. The title bar and tab strip, the file list's sort chevron, hover, selection and focus ring, and the menus' rounded corners, spacing and fade-in are drawn as in Windows 11. The classic menu shows a bold Open and no shortcut text, menu rows show no tooltips, and the Back, Forward, Up and Refresh tooltips name where they lead. File types are named as Windows names them ("ODS Spreadsheet", "PNG File").
* **Command bar and status bar:** "…", View and This PC's commands are laid out as in Windows 11, and the Recycle Bin has Empty Recycle Bin and Restore all items there. The status bar reads like Windows 11's: the item count, the selection and its size, and the Details and Large icons toggles.
* **Views:** new Tiles and Content views (Ctrl+Shift+7 and Ctrl+Shift+8), which keep a folder's groups. Groups collapse and expand under their headings in Details, Tiles and Content. In Details, a click anywhere on a row selects it, a rubber band can start beside a name, and Ctrl+plus on the number pad or the column titles' menu sizes the columns to fit. New users start with the Details pane closed and en-US dates written as 9/13/2026.
* **Details pane:** laid out as in Windows 11: the picture, the name, Share, Properties and the file's details, photo details included.
* **Navigation pane:** in Windows 11's order: Home, the pinned folders, then This PC and Network. This PC, Network and drives have Windows 11's right-click menus, and the Map network drive button below the pane is gone (it stays in those menus and in "…"). **Recent files** now opens, and its rows offer Open file location.
* **This PC and Network:** This PC shows drives and network locations as tiles with a capacity bar, in collapsible groups. Network lists the computers on your network as items, with an info bar when discovery is off. A drive can be copied and pasted or dragged into a folder as a shortcut, as in Windows. Map network drive, Enter network credentials and Disconnect use Windows 11's layout and words, and Map network drive offers only free drive letters.
* **Address bar and keys:** a chevron before the crumbs lists the top-level places, a long path folds its first crumbs into «, and a crumb's menu lists every subfolder. The keyboard reaches the « and the chevron, and every Tab stop shows where the keyboard is. Address bar suggestions have 40 px rows and show the current one with a light fill and a blue bar; Up and Down wrap back to what you typed, and Escape puts it back. F3 goes to search, Space only selects, and scrolling over the address bar no longer changes folder. The file list's focused item is outlined only while the list has the keyboard.
* **Menus and dragging:** right-click menus are laid out as in Windows 11 24H2: the top strip's Cut, Copy, Rename, Share and Delete buttons carry labels, with thin lines between them, and the strip sits on the side nearest the pointer; Settings › Appearance › Buttons in the right-click menu can keep it always at the top or the bottom instead. This PC's group headings have Collapse group, Expand all groups and Collapse all groups, as the file list's do. Submenus fly out to the side on hover. Show more options has Send to: a compressed folder, a shortcut on the desktop, Documents, a mail recipient, drives and Bluetooth devices. A right-button drag asks Copy here, Move here or Create shortcuts here, and a drag says what a drop will do ("Move to Documents"). New lists Windows 11's items and creates each file at once, ready to rename. Open with chooses an app as Windows 11 does: the current default, suggested apps, More apps, Just once and Always.
* **Copying, moving and deleting:** Delete sends items to the Recycle Bin without asking, and a permanent delete asks Windows' Yes or No question. Copy, cut and drag take up to 100,000 items, and a folder held under a drag no longer opens. The progress window asks Replace or Skip Files, with a File Conflict window to compare both files, and asks about every failed item in Windows 11's words, with nothing reported afterwards; the Operation result dialog is gone. Compress and extract show their progress there too. A new setting lets Replace overwrite the old file as Windows does; by default it still goes to the Recycle Bin first, and cancelling the copy brings it back.
* **Renaming:** the part of the name Windows 11 selects is selected, shortcuts are treated as Windows treats them, Windows' forbidden characters are refused as you type, and a taken name offers "name (2)".
* **Properties:** every Properties window has Windows 11's fixed size and 9-point text. Files gain a Details tab, local folders the whole Customize tab (folder templates, a folder picture and Change Icon's grid of icons), drives Windows 11's used and free space ring without counting their files, and several items show their names, Read-only and Hidden. A folder that is a mount point shows as a folder, and Read-only stays in step with a change made on the Security tab.
* **Folder Options:** its own compact window, with Windows 11's General, View and Search tabs: Browse folders, Privacy, the View tree and the search options.
* **Open and Save dialogs:** laid out as Windows lays them out. The name is drawn selected only while it has the keyboard, and a window opened from inside a dialog belongs to it.
* **Recycle Bin:** a restore that meets a taken name asks Replace or Skip Files, also when two items go back to the same name.
* **Search:** results are a step in the tab's history ("Search Results in Documents"), so Back and Forward move between a folder and its results, also in a middle-clicked tab, and files whose text matches are kept.
* **Thumbnails and clipboard:** the desktop's own thumbnailers make previews of videos and other files, also when installed outside `/usr` or on NixOS, and pictures no gdk-pixbuf loader declares get previews. Paste writes a picture on the clipboard, such as a screenshot, as a new image file, and Copy and Cut also put the items' paths on the clipboard as text; Settings › Files & folders › Clipboard can turn either off. Open in Terminal can start kitty, and pasting into a folder opened through a symbolic link works.
* **Fixes:** the file list is drawn again after "Working on it..." on a slow share, instead of staying blank. A hovered submenu no longer opens after the keyboard moves on. Select all, Select none and Invert selection work on This PC's drives. A template given to subfolders is kept when only the folder changes later. Tiles and Content write sizes as the status bar does.

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.15_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.15-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.15-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.15-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.15-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.14...v2.0.15

# 2.0.14 — 2026-11-05

## Security
* Deleting a whole folder permanently while using **Open as administrator** no longer goes through GVfs, which deleted it by path as root. OpenXplorer now asks for administrator authentication and deletes it with its own small helper, which holds each folder open while it empties it, so another user who swaps a folder inside it for a shortcut cannot redirect the deletion, and nothing on another drive is entered. This replaces the refusals in 2.0.12 and 2.0.13: folders in a user's home, `/tmp`, `/var/tmp`, group-writable folders such as `/var/log`, and folders on removable drives, drives other users can mount and network shares can be deleted this way again. A drive mounted inside the folder is left in place and named. A shortcut on the way to the folder is followed only when it belongs to the administrator on the system drive and nobody else can change the folders above it. In the Flatpak, a whole folder still can't be deleted under **Open as administrator**. Single files and links are deleted as before.

## What's Changed
* fix(tabs): Don't crash on Wayland when a tab is dragged out quickly by @AKolenda in https://github.com/AKolenda/openxplorer/pull/199
* feat(updates): Pop up new releases, show Update in the bars and bring back the Settings gear by @AKolenda in https://github.com/AKolenda/openxplorer/pull/198
* fix(focus): Return the keyboard to the file list after navigating or editing the address by @clansty in https://github.com/AKolenda/openxplorer/pull/197
* fix(ops): Delete admin:// folders through a pinned walk, building on #192 by @addition-official in https://github.com/AKolenda/openxplorer/pull/196

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.14_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.14-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.14-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.14-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.14-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.13...v2.0.14

# 2.0.13 — 2026-11-03

## Security
* Deleting a folder permanently while using **Open as administrator** is now also refused, before anything is deleted, when the folder or a folder on the way to it is on a drive an ordinary user can mount or plug in, wherever that drive is mounted, or when such a drive can be mounted on the way to the folder or inside it. That covers drives with a `user`, `users`, `owner`, `group` or `x-udisks-auth` entry in `/etc/fstab`, drives a user mounted, and removable, USB, SD card, FireWire, Thunderbolt and loop devices. Internal drives mounted through `/etc/fstab`, the root filesystem and a separate `/home` partition are unaffected.

## What's Changed
* fix(ops): Refuse administrator folder deletes on drives a user can mount by @AKolenda in https://github.com/AKolenda/openxplorer/pull/194

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.13_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.13-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.13-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.13-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.13-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.12...v2.0.13

# 2.0.12 — 2026-11-03

## Security
* Deleting a folder permanently while using **Open as administrator** is now refused, before anything is deleted, when other user accounts can change that folder, a folder inside it or a folder on the way to it, when it is on a drive whose permissions can't be checked, or when a drive is mounted at or inside it. Single files and links are deleted as before. This means some folders can no longer be deleted this way, such as folders in a user's home, `/tmp` and other shared or group-writable system folders, and folders on removable drives; in the Flatpak, every folder delete under **Open as administrator** is refused.
* A network share that another application asks OpenXplorer to show (Show in folder) is no longer connected to on its own: its tab says the share is not mounted until you choose **Try again**.

## What's Changed
* feat(look): fix the selection band, open ZIPs like folders and match Windows 11 Explorer's menus and defaults by @AKolenda in https://github.com/AKolenda/openxplorer/pull/183
* fix(ops): Claim the transfer panel in one place that refuses a busy window by @addition-official in https://github.com/AKolenda/openxplorer/pull/176
* fix(details): Hide folders' item counts as soon as the setting is turned off by @addition-official in https://github.com/AKolenda/openxplorer/pull/175
* fix(clipboard): Quote each path in Copy as path, as Windows does by @addition-official in https://github.com/AKolenda/openxplorer/pull/168
* chore(deps): bump next from 16.4.2 to 16.4.3 by @dependabot[bot] in https://github.com/AKolenda/openxplorer/pull/171
* feat(new): Create a text document at once and rename it in place by @addition-official in https://github.com/AKolenda/openxplorer/pull/174
* fix(search): Reuse SQLite connections so the WAL stays in place by @clansty in https://github.com/AKolenda/openxplorer/pull/187
* fix(dialogs): Save to network folders and remember each app's last folder by @addition-official in https://github.com/AKolenda/openxplorer/pull/178
* fix(integration): Open Show in folder by the setting, at the share's address by @addition-official in https://github.com/AKolenda/openxplorer/pull/180
* chore(deps): bump crc32fast from 1.5.2 to 1.5.3 in /native by @dependabot[bot] in https://github.com/AKolenda/openxplorer/pull/159
* chore(deps): bump rustix from 1.1.5 to 1.1.6 in /native by @dependabot[bot] in https://github.com/AKolenda/openxplorer/pull/170
* chore(deps): bump lzma-rust2 from 0.22.0 to 0.24.0 in /native by @dependabot[bot] in https://github.com/AKolenda/openxplorer/pull/172
* perf(search): Keep each root's entry count instead of counting it by @clansty in https://github.com/AKolenda/openxplorer/pull/186
* perf(search): Skip build output and installed dependencies when indexing by @clansty in https://github.com/AKolenda/openxplorer/pull/188
* feat(rename): Rename several selected items at once, as Windows Explorer does by @addition-official in https://github.com/AKolenda/openxplorer/pull/184
* fix(view): Show at once that a slow location is being opened, as Explorer does by @addition-official in https://github.com/AKolenda/openxplorer/pull/185
* chore(deps-dev): bump wrangler from 4.151.0 to 4.154.0 by @dependabot[bot] in https://github.com/AKolenda/openxplorer/pull/173
* fix(tabs): Drag tabs as Chrome does on Wayland, keeping the window still by @addition-official in https://github.com/AKolenda/openxplorer/pull/177
* feat(properties): Redo Properties as Windows 11's Properties windows by @addition-official in https://github.com/AKolenda/openxplorer/pull/179
* feat(transfers): Show running operations in a Windows-style progress window by @addition-official in https://github.com/AKolenda/openxplorer/pull/190
* fix(ops): Refuse administrator deletes of folders other users can change by @AKolenda in https://github.com/AKolenda/openxplorer/pull/192
* fix(integration): Don't connect to a share opened over D-Bus until asked by @AKolenda in https://github.com/AKolenda/openxplorer/pull/191

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.12_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.12-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.12-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.12-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.12-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.11...v2.0.12

# 2.0.11 — 2026-11-02

## What's Changed
* fix(search): stop shown search results from refreshing while the cache changes by @AKolenda in https://github.com/AKolenda/openxplorer/pull/181

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.11_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.11-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.11-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.11-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.11-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.10...v2.0.11

# 2.0.10 — 2026-10-30

## What's Changed
* feat(i18n): Add a Simplified Chinese translation, keeping English the default by @AKolenda in https://github.com/AKolenda/openxplorer/pull/166 (translation by @clansty, from https://github.com/AKolenda/openxplorer/pull/165)

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.10_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.10-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.10-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.10-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.10-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.9...v2.0.10

# 2.0.9 — 2026-10-29

## What's Changed
* fix(rename,views): keep the rename field open and turn item check boxes off by default by @AKolenda in https://github.com/AKolenda/openxplorer/pull/162
* fix(search): stop a word found only in the extension from matching by @AKolenda in https://github.com/AKolenda/openxplorer/pull/164

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.9_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.9-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.9-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.9-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.9-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.8...v2.0.9

# 2.0.8 — 2026-10-23

## What's Changed
* fix(properties,shortcuts): follow up Codex review on Size on disk and Super+E records by @addition-official in https://github.com/AKolenda/openxplorer/pull/136
* fix(menus): show submenus in the same popup as their menu by @addition-official in https://github.com/AKolenda/openxplorer/pull/135
* fix(files): do nothing when a cut item is pasted into its own folder by @addition-official in https://github.com/AKolenda/openxplorer/pull/134
* revert(devices): remove the read-only drive notice and greyed-out commands from #71 by @addition-official in https://github.com/AKolenda/openxplorer/pull/137
* revert(notifications): stop sending a desktop notification when a file operation ends by @addition-official in https://github.com/AKolenda/openxplorer/pull/139
* fix(keys): keep Delete away from files behind an open dialog by @addition-official in https://github.com/AKolenda/openxplorer/pull/151
* revert(ui): stop showing confirmation messages Windows Explorer does not show by @addition-official in https://github.com/AKolenda/openxplorer/pull/141
* fix(rename): cancel a pending slow-click rename on commands and keep it on the clicked item by @addition-official in https://github.com/AKolenda/openxplorer/pull/153
* fix(transfers): name the real file when a copy fails by @addition-official in https://github.com/AKolenda/openxplorer/pull/146
* feat(keys): add Explorer's Ctrl+E, Ctrl+D, F11 and Alt+P by @addition-official in https://github.com/AKolenda/openxplorer/pull/144
* revert(menus): keep New and Paste on in folders you cannot write to by @addition-official in https://github.com/AKolenda/openxplorer/pull/138
* fix(delete): move items to the Recycle Bin wherever it works by @addition-official in https://github.com/AKolenda/openxplorer/pull/145
* fix(ops): show no result dialog after the user cancels by @addition-official in https://github.com/AKolenda/openxplorer/pull/147
* fix(menus): show menus in full when they fit on the monitor by @addition-official in https://github.com/AKolenda/openxplorer/pull/148
* feat(share): make the command bar Share button email the selected files by @addition-official in https://github.com/AKolenda/openxplorer/pull/149
* feat(views): replace Adjust view display style with Windows Folder Options by @addition-official in https://github.com/AKolenda/openxplorer/pull/150
* revert(ops): stop showing a message and Undo button after a file operation by @addition-official in https://github.com/AKolenda/openxplorer/pull/140

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.8_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.8-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.8-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.8-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.8-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.7...v2.0.8

# 2.0.7 — 2026-10-23

## What's Changed
* feat(defaults): include other apps' Open and Save dialogs in Make OpenXplorer default by @AKolenda in https://github.com/AKolenda/openxplorer/pull/132
* fix(native,web): resolve safety, correctness, and performance bugs by @AKolenda in https://github.com/AKolenda/openxplorer/pull/126
* feat(devices): format drives in OpenXplorer's own Format dialog by @AKolenda in https://github.com/AKolenda/openxplorer/pull/133
* feat(archives): press Enter in the Extract dialog to extract by @AKolenda in https://github.com/AKolenda/openxplorer/pull/142
* chore(web): Upgrade Next.js to 16.4.2 by @AKolenda in https://github.com/AKolenda/openxplorer/pull/152
* perf(delete): Ask a share once per file instead of three times when deleting by @AKolenda in https://github.com/AKolenda/openxplorer/pull/154
* feat(defaults): send GTK 3 apps' Open and Save dialogs to OpenXplorer by @AKolenda in https://github.com/AKolenda/openxplorer/pull/143

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.7_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.7-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.7-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.7-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.7-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.6...v2.0.7

# 2.0.6 — 2026-10-13

## What's Changed
* ci(checks): validate website and dependency changes before merge by @AKolenda in https://github.com/AKolenda/openxplorer/pull/108
* fix(web): render standalone designs with the pinned compiler by @AKolenda in https://github.com/AKolenda/openxplorer/pull/110
* fix(release): preserve package inputs and check publication data by @AKolenda in https://github.com/AKolenda/openxplorer/pull/109
* fix(capture): run native builds from home-directory checkouts by @AKolenda in https://github.com/AKolenda/openxplorer/pull/111
* fix(archives): bound decoder memory for compressed files by @AKolenda in https://github.com/AKolenda/openxplorer/pull/112
* fix(launchers): bound metadata reads during file dragging by @AKolenda in https://github.com/AKolenda/openxplorer/pull/114
* fix(mounts): scope rollback to owned setup changes by @AKolenda in https://github.com/AKolenda/openxplorer/pull/115
* fix(search): cap listed matches before showing results by @AKolenda in https://github.com/AKolenda/openxplorer/pull/116
* perf(native): reduce cache, sorting and progress overhead by @AKolenda in https://github.com/AKolenda/openxplorer/pull/113
* feat(dialogs): make Brave's downloads, Save as PDF and uploads reach OpenXplorer's dialogs by @AKolenda in https://github.com/AKolenda/openxplorer/pull/107
* feat(views,search): Ctrl-only item check boxes and experimental frequent-folder indexing by @AKolenda in https://github.com/AKolenda/openxplorer/pull/106

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.6_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.6-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.6-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.6-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.6-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.5...v2.0.6

# 2.0.5 — 2026-10-12

## What's Changed
* fix(ci): stop waiting 10 s on zombie processes after each test binary by @AKolenda in https://github.com/AKolenda/openxplorer/pull/90
* feat(drag): move a plain drag within a drive and copy it across drives by @addition-official in https://github.com/AKolenda/openxplorer/pull/91
* feat(properties): show Size on disk like Windows Explorer by @addition-official in https://github.com/AKolenda/openxplorer/pull/93
* feat(rename): ask before a rename changes a file's extension by @addition-official in https://github.com/AKolenda/openxplorer/pull/94
* feat(integration): let Super+E open OpenXplorer on KDE Plasma by @addition-official in https://github.com/AKolenda/openxplorer/pull/95
* feat(files): make a new folder at once and rename it in place, as Windows does by @addition-official in https://github.com/AKolenda/openxplorer/pull/100
* fix(menus): disable New and Paste in a folder you cannot write to by @addition-official in https://github.com/AKolenda/openxplorer/pull/101
* fix(views): make Apply to all folders apply the choices in the dialog by @addition-official in https://github.com/AKolenda/openxplorer/pull/103
* fix(notifications): make Show after a delete open the Recycle Bin by @addition-official in https://github.com/AKolenda/openxplorer/pull/102
* chore(deps): bump next from 16.3.8 to 16.4.0 by @dependabot[bot] in https://github.com/AKolenda/openxplorer/pull/96
* chore(deps-dev): bump wrangler from 4.147.0 to 4.148.0 by @dependabot[bot] in https://github.com/AKolenda/openxplorer/pull/97
* feat(files): name copies like Windows Explorer ("name - Copy", "name - Copy (2)") by @addition-official in https://github.com/AKolenda/openxplorer/pull/92
* fix(views): let Apply to all and Reset cover Downloads' Group by by @addition-official in https://github.com/AKolenda/openxplorer/pull/104
* fix(views): let an empty folder take right-click, paste and drops by @addition-official in https://github.com/AKolenda/openxplorer/pull/98
* feat(views): show an empty folder as Windows Explorer does by @addition-official in https://github.com/AKolenda/openxplorer/pull/99

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.5_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.5-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.5-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.5-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.5-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.4...v2.0.5

# 2.0.4 — 2026-10-09

## What's Changed
* ci(repo): Require Conventional Commits pull request titles by @AKolenda in https://github.com/AKolenda/openxplorer/pull/78
* fix(dialogs): keep shares from blocking, say why Save can't save, filter both panes, guard Close by @addition-official in https://github.com/AKolenda/openxplorer/pull/73
* chore(deps-dev): bump wrangler from 4.145.0 to 4.147.0 by @dependabot[bot] in https://github.com/AKolenda/openxplorer/pull/79
* ci(repo): Require a Model line in PRs and protect the release runner and Cloudflare token by @AKolenda in https://github.com/AKolenda/openxplorer/pull/83
* ci(release): Retry failed release and deploy jobs on main by @AKolenda in https://github.com/AKolenda/openxplorer/pull/84
* fix(views): keep a folder's expand arrow inside its row's highlight by @addition-official in https://github.com/AKolenda/openxplorer/pull/86
* fix(views): show item check boxes as Windows 11 does by @addition-official in https://github.com/AKolenda/openxplorer/pull/85
* feat(settings): lay out the Settings page anew by @addition-official in https://github.com/AKolenda/openxplorer/pull/82
* fix(security): run a dropped-on file only when it really is a program by @addition-official in https://github.com/AKolenda/openxplorer/pull/89 (GHSA-9x68-px3w-xgg8)

## Packages
Download them from the assets below.

| Package | For |
| --- | --- |
| `openxplorer_2.0.4_all.deb` | Zorin OS 18, Ubuntu 24.04 and newer, Debian 13 (OpenXplorer 1.1.x offers it in **Check for updates**) |
| `openxplorer-2.0.4-1.fc*.x86_64.rpm` | Fedora |
| `openxplorer-2.0.4-1.opensuse_tumbleweed.x86_64.rpm` | openSUSE Tumbleweed |
| `openxplorer-2.0.4-1-x86_64.pkg.tar.zst` | Arch Linux |
| `io.winspace.Development.flatpak` | Any distribution with Flatpak |
| `openxplorer-2.0.4-source.zip` | Corresponding source (AGPL-3.0-only) |
| `SHA256SUMS` | Checksums of every file above |

**Full Changelog**: https://github.com/AKolenda/openxplorer/compare/v2.0.3...v2.0.4

# 2.0.3 — 2026-10-06

Windows Explorer's Group by, safer copies and archives, drives that are
read-only or gone, and Open and Save dialogs that no longer freeze or get
stuck.

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
- Choosing an item in a side menu, such as Sort > Group by > Date
  modified, closes every menu, and a click anywhere in the window closes
  any menu still open. On KDE Plasma the Sort menu could stay on screen
  until a window of another app was clicked.
- An Open or Save dialog can no longer get stuck with Save, Cancel and
  Escape doing nothing:
  - "Replace it?" is asked inside the dialog, not in a window of its own
    that could open out of sight and take every click and key.
  - The dialog answers only once its window really closes. A window that
    refused to close (an update installing, files being written in it)
    used to keep a dead dialog after answering; now it says why and the
    dialog stays usable.
  - No tab can be moved into a dialog, which made closing it ask about
    tabs.
  - Escape cancels with Caps Lock or Num Lock on.
  - Opening a window no longer waits for the Recycle Bin's watch, which
    could hold the whole app, an open dialog included, while a network
    share or drive stopped answering.
- An Open or Save dialog opened by another application now belongs to that
  application's window on Wayland, as Windows' dialog and KDE's own do: it
  stays above the window, and the window cannot be used until the dialog
  is answered or cancelled. The other OpenXplorer windows stay usable. This
  needs a compositor with xdg-foreign and xdg-dialog (KDE Plasma 6.1 or
  newer) and GTK 4.22 or newer; under X11 the dialog is a window of its
  own, as before.
- A drive Linux mounted read-only, such as the Windows drive of a
  dual-boot computer while Windows is hibernated or used Fast startup,
  is noticed: New, Paste, Cut, Rename, Duplicate and Delete are turned
  off there and say why, Copy still works, and the status bar shows
  "Read-only drive" with what to do (shut Windows down fully, then mount
  the drive again). A write that still fails there says the drive is
  read-only instead of the bare "Read-only file system".
- Copying to a Windows drive (NTFS), or to a FAT or exFAT stick, no
  longer creates names Windows cannot use. A device name such as CON,
  PRN, AUX, NUL, COM1 or LPT1 (also with an extension, like `nul.txt`)
  and a name ending in a dot or a space are asked about like names with
  forbidden characters: Rename gives the item a name Windows opens
  (`_nul.txt`, `notes_`), Skip leaves it out, and "Do this for all such
  items" covers the rest of the copy.
- A tab showing a USB drive that was unplugged, or a share or disk that
  was unmounted by another program, no longer keeps showing the old
  files. Every tab and split pane on it drops them and says "This
  location is unavailable: the drive or network share that holds this
  folder was disconnected", with Try again, which lists the folder once
  the drive or share is back.
- The Checksums tab no longer hangs on a named pipe (FIFO): a pipe, a
  device or a socket is refused at once with a message, since it has no
  contents to sum and reading a pipe waits forever for a writer, which
  Cancel could not stop. Closing a window now also stops its folder-size
  scan and the work of its Properties dialogs, which kept running.
- Copying or moving a folder no longer fails as a whole when one file
  inside it cannot be read (a locked or protected file, a socket, a pipe
  or another special file). As in Windows Explorer, OpenXplorer asks about
  that file by its path inside the folder: Retry copies only it again,
  Skip or Skip all leave only it out, and the rest of the folder is
  copied. The files left out are listed at the end, and a move keeps them,
  with their folders, where they were.
- Copying files out of a ZIP opened like a folder no longer fails because
  of one bad item elsewhere in the ZIP, such as a symbolic link: only the
  items you copy are checked. A link or special file inside a copied
  folder, which the folder view hides, is left out, and the message says
  so. Extract all still refuses a ZIP with such items.
- TAR archives get the ZIP safety limits they were missing. A compressed
  TAR that unpacks to over 1,000 times its size (a "TAR bomb") is refused
  before anything is extracted, as such a ZIP is: each file counts its
  share of the archive's compressed size. A crafted size near the largest
  number a header can hold is a damaged archive instead of a crash in
  debug builds. And a TAR's file names are kept only up to 32 MiB, as for
  a ZIP's directory, so a 1.3 MB archive of long names no longer takes
  2.4 GB of memory to list; it says its names are too long instead.
- A file list at its top stays at its top when files come before the one
  at the top edge, in every view: in an Open or Save dialog switching
  from one file type to more (`*.svg`, then All files) no longer scrolls
  the list down past its first files. GTK kept the row that was at the
  top edge there. A position restored by Back or a tab switch, a file the
  window scrolls to and a list scrolled down by hand are left alone.
- Folder views, as in Windows' Folder Options: the view display style
  dialog (View > Adjust view display style…) has "Apply to all folders",
  which makes every folder show the current folder's view and forget its
  own, and "Reset folders", which makes every folder forget its view and
  show the default one. Both ask first, and every open window follows at
  once.
- Open and Save dialogs no longer freeze the app when their folder is on a
  network share that stopped answering. The dialog checks the caller's
  folder, typed names and files it would replace off the main thread and
  waits at most three seconds: a folder that does not answer opens the
  dialog in the home folder, and Save or Open says the share is not
  answering. A file opened from such a share is sent as read-only.
- The right-click menu of a This PC or Network card no longer crashes the
  app when the page is drawn again while it is open (a drive or share
  connecting, a server being found, or the menu's own Remove or Sign out).
- Deleting permanently (Shift+Delete, or Delete where there is no Recycle
  Bin) never goes into another drive. A folder where a drive, share or bind
  mount is mounted is refused with "… is where a drive or share is mounted",
  and a folder with one mounted inside it is refused before anything is
  deleted, naming the mount. Before, the deletion went into the mounted
  drive, deleted its files and only failed at the end with "Device busy".
- Settings' small grey text is no longer cut off at the top or bottom
  (#34). With GTK 4.22's Vulkan and NGL renderers, Segoe UI drawn at a
  fractional size such as 12.5 pixels could lose the tops or bottoms of
  its letters. Settings' descriptions and notes are now 12 pixels and its
  titles 14, and every font size is rounded to a whole pixel at every text
  size.

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
