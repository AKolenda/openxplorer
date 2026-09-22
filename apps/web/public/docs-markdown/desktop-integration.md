# Default file manager

Make explicit changes, with a way back.

## Set folder and SMB handlers

Settings → Default file explorer → Make OpenXplorer default changes the per-user folder and SMB associations and records previous handlers. Installing the package does not do this for you.

## Brave: Show in folder

Opening the ZIP filename in Brave and Show in folder are different actions. ZIP opening uses an archive association; Show in folder may use a desktop portal, FileManager1, or the default directory handler. Folder defaults alone do not prove either browser route is configured.

Settings shows separate statuses for folders, SMB links, ZIP files and the current FileManager1 owner. Keep Include Brave / other apps’ Show in folder integration checked when applying folder defaults. If ownership says waiting for another file manager, finish operations and close that app; log out and back in if necessary. OpenXplorer does not kill it.

Restart Brave after changing handlers. Test Show in folder checks FileManager1, not Brave’s portal. A browser portal can retain a different choice; select OpenXplorer in its chooser when available. Do not disable the system portal or replace upload/save file-picker dialogs.

## Open ZIP downloads in OpenXplorer

In Settings → Default file explorer, click Use OpenXplorer for ZIPs. This is an explicit, per-user change to ZIP associations, with a Restore ZIP handler button. It does not change PDF, video, or document defaults. Installation never changes these associations.

A ZIP opens in the built-in ZIP browser; this setting does not automatically extract it. You can instead leave ZIPs assigned to an external archive manager.

```sh
xdg-mime query default inode/directory
xdg-mime query default application/zip
openxplorer --diagnose
```

## What this does not replace

Browsers can route reveal actions through desktop portals or keep a previous application choice. Choose OpenXplorer in a chooser when available. Upload/save file-picker dialogs stay with the system. Super+E is a separate desktop keyboard shortcut; the app does not override it.

## Tabs, windows and the taskbar

Drag a tab onto another OpenXplorer window’s tab strip to merge it, or drop it outside to create a window. You can also right-click the tab and choose Move tab to window… to select an existing destination. The original is kept until the destination accepts the tab. Close this tab’s dialog and finish file operations first.

The installed launcher offers New window, Open windows and Settings. File dragging uses its own native GTK transport: drop selected files into compatible applications, or into an OpenXplorer folder to confirm a copy. Ctrl+C/Ctrl+X/Ctrl+V remain available, including GNOME and KDE file clipboards. Native Wayland and individual application behavior need confirmation on the target desktop.

## Restore the previous setup

Disable Show in folder removes only OpenXplorer’s unmodified user-level service files. Restore previous also restores recorded file associations. Modified or third-party configuration is preserved for manual review.

---

OpenXplorer 1.1.3. Project-authored documentation: AGPL-3.0-only.
