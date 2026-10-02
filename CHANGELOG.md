# Unreleased

- Optional: other applications' Open and Save dialogs in OpenXplorer.
  Settings > Default apps > "Apps' Open and Save dialogs" > Enable makes
  applications that use the desktop portal (Chrome, Firefox, Flatpak apps)
  choose and save files in an OpenXplorer window, with File name, the type
  list and Save or Open at the bottom. It is off until enabled, keeps every
  other portal backend, and Restore Open and Save dialogs undoes it. Host
  packages install `/usr/share/xdg-desktop-portal/portals/<id>.portal`;
  the Flatpak cannot offer it.

- Extract all… works like Windows Explorer's: one field, "Files will be
  extracted to this folder", filled in with the ZIP's folder and name, with
  Browse…. A missing folder is created; an existing one (such as
  Downloads) receives the files directly, asking before any file is
  replaced; a ZIP holding one folder of the same name is no longer nested
  (`tidewater/tidewater`).
- Open in archive manager no longer reopens the archive in OpenXplorer when
  OpenXplorer is the default application for ZIPs.
- The Extract dialog is as short as Explorer's: the folder field, Browse…,
  "Show extracted files when finished", Cancel and Extract. The counts,
  notes and Open in archive manager are behind the (i) button. A
  password-protected ZIP now says so straight away and offers the archive
  manager, instead of "Wait for the ZIP check to finish".
- ZIPs can open like folders, as in Windows Explorer: Settings > Windows &
  tabs > Open ZIP files > Like a folder (Windows). The ZIP opens in the tab,
  with the address bar, crumbs, Back and Up working through it, Extract all
  in the bar, and Copy, Paste and dragging items out (as real copies). It
  stays read-only. The default, In a pop-up window, keeps today's window.
- Extract all appears in the command bar while a ZIP is selected, as in
  Windows Explorer.

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
