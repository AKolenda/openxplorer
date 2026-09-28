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
- The Python app in `desktop/` is deprecated and no longer shipped.

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

Earlier releases: [desktop/CHANGELOG.md](desktop/CHANGELOG.md).
