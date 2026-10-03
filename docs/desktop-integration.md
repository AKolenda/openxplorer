# Default file manager

Make explicit changes, with a way back.

## Set folder and SMB handlers

Settings → Default file explorer → Make OpenXplorer default changes the per-user folder and SMB associations and records previous handlers. Installing the package does not do this for you.

## Brave: Show in folder

Opening the ZIP filename in Brave and Show in folder are different actions. ZIP opening uses an archive association; Show in folder may use a desktop portal, FileManager1, or the default directory handler. Folder defaults alone do not prove either browser route is configured.

Settings shows separate statuses for folders, SMB links, ZIP files and the current FileManager1 owner. Keep Include Brave / other apps’ Show in folder integration checked when applying folder defaults. If ownership says waiting for another file manager, finish operations and close that app; log out and back in if necessary. OpenXplorer does not kill it.

Restart Brave after changing handlers. Test Show in folder checks FileManager1, not Brave’s portal. A browser portal can retain a different choice; select OpenXplorer in its chooser when available. Do not disable the system portal. To pick and save files in OpenXplorer too, see Open and Save dialogs below.

## Open ZIP downloads in OpenXplorer

In Settings → Default file explorer, click Use OpenXplorer for ZIPs. This is an explicit, per-user change to ZIP associations, with a Restore ZIP handler button. It does not change PDF, video, or document defaults. Installation never changes these associations.

A ZIP opens in the built-in ZIP browser; this setting does not automatically extract it. You can instead leave ZIPs assigned to an external archive manager.

```sh
xdg-mime query default inode/directory
xdg-mime query default application/zip
openxplorer --diagnose
```

## Open and Save dialogs

Settings → Default apps → Apps’ Open and Save dialogs → Enable makes other applications choose and save files in an OpenXplorer window: the same navigation pane, address bar and search, with File name, Save as type and the Save or Open button at the bottom. It works for applications that use the desktop portal for file dialogs, such as Chrome, Firefox and Flatpak apps; applications that draw their own dialog keep it.

Enabling writes one preference into your own desktop-portal configuration (the file the portal reads now, or a new ~/.config/xdg-desktop-portal/<desktop>-portals.conf) and records what the file held; every other portal, such as screenshots or screen sharing, keeps its backend. Click Apply now to restart the desktop portal when it runs as a user service, or log out and back in. Saving over an existing file always asks first.

On KDE Plasma, KDE’s own apps (Plasma and its widgets, Kate, System Settings and the rest) show their own dialog unless told to use the portal, so Enable also adds ~/.config/plasma-workspace/env/openxplorer-file-dialogs.sh, which sets PLASMA_INTEGRATION_USE_PORTAL=1 when you log in. KDE apps follow after you log out and back in; Restore removes the file again. If a file of yours already has that name, OpenXplorer leaves it alone and the status line says KDE apps keep KDE’s dialog.

Restore Open and Save dialogs puts the file back as it was, or, if you edited it since, removes only OpenXplorer’s line. The Flatpak cannot change the host’s portal, so this is available in the installed package only.

```sh
cat ~/.config/xdg-desktop-portal/*-portals.conf
systemctl --user try-restart xdg-desktop-portal.service
```

## What this does not replace

Browsers can route reveal actions through desktop portals or keep a previous application choice. Choose OpenXplorer in a chooser when available. Upload/save file-picker dialogs stay with the system unless you turn on Open and Save dialogs. Super+E is a separate desktop keyboard shortcut; the app does not override it.

## Tabs, windows and the taskbar

Drag a tab onto another OpenXplorer window’s tab strip to merge it, or drop it outside to create a window. You can also right-click the tab and choose Move tab to window… to select an existing destination. The original is kept until the destination accepts the tab. Close this tab’s dialog and finish file operations first.

The installed launcher offers New window, Open windows and Settings. File dragging uses its own native GTK transport: drop selected files into compatible applications, or into an OpenXplorer folder to confirm a copy. Ctrl+C/Ctrl+X/Ctrl+V remain available, including GNOME and KDE file clipboards. Native Wayland and individual application behavior need confirmation on the target desktop.

## Restore the previous setup

Disable Show in folder removes only OpenXplorer’s unmodified user-level service files. Restore previous also restores recorded file associations. Modified or third-party configuration is preserved for manual review. Restore Open and Save dialogs gives file dialogs back to the desktop the same way.

---

OpenXplorer 2.0.0. Project-authored documentation: AGPL-3.0-only.
