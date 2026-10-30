# Default file manager

Make explicit changes, with a way back.

## Set folder and SMB handlers

Settings → Default apps → Make OpenXplorer default changes the per-user folder and SMB associations and records previous handlers. With Also handle Open and Save dialogs, which starts on, it also sends other apps’ file pickers, such as Brave’s uploads and Save dialogs, to OpenXplorer (see Open and Save dialogs below); click Apply now beside Open and Save dialogs, or log out and back in, for them to follow. If the card says OpenXplorer is your default file explorer, except for Open and Save dialogs, click Make OpenXplorer default again with that option on. Installing the package does not do this for you.

## Brave: Show in folder

Opening the ZIP filename in Brave and Show in folder are different actions. ZIP opening uses an archive association; Show in folder may use a desktop portal, FileManager1, or the default directory handler. Folder defaults alone do not prove either browser route is configured.

Settings shows separate statuses for folders, SMB links, ZIP files and the current FileManager1 owner. Keep Include Brave / other apps’ Show in folder integration checked when applying folder defaults. If ownership says waiting for another file manager, finish operations and close that app; log out and back in if necessary. OpenXplorer does not kill it.

Restart Brave after changing handlers. Test Show in folder checks FileManager1, not Brave’s portal. A browser portal can retain a different choice; select OpenXplorer in its chooser when available. Do not disable the system portal. To pick and save files in OpenXplorer too, see Open and Save dialogs below.

## Open ZIP downloads in OpenXplorer

In Settings → ZIP & archives, click Use OpenXplorer for ZIPs. This is an explicit, per-user change to ZIP associations; Settings → Default apps → Undo and troubleshooting has the Restore ZIP handler button. It does not change PDF, video, or document defaults. Installation never changes these associations.

A ZIP opens in the built-in ZIP browser; this setting does not automatically extract it. You can instead leave ZIPs assigned to an external archive manager.

```sh
xdg-mime query default inode/directory
xdg-mime query default application/zip
openxplorer --diagnose
```

## Open and Save dialogs

Settings → Default apps → Other apps' Open and Save dialogs → Enable, or Make OpenXplorer default with Also handle Open and Save dialogs on, makes other applications choose and save files in an OpenXplorer window: the same navigation pane, address bar and search, with File name, Save as type and the Save or Open button at the bottom. It works for applications that use the desktop portal for file dialogs, such as Chrome, Brave, GTK 4 apps and Flatpak apps, and, from your next login, GTK 3 apps (see below); applications that draw their own dialog keep it. As in Windows, the File name box takes a name, a path from the folder shown, ~/… or a full path: a folder opens, a file is chosen, and Save adds the chosen type’s extension unless the name has it already or ends in another known type’s (so “Notes v2.1” is saved as “Notes v2.1.pdf”); choosing another Save as type changes the name’s extension to the new type’s. A file typed in the address bar is chosen too. Escape and Ctrl+Q cancel the dialog, Alt+Left, Alt+Right and Alt+Up work from the File name box, and Ctrl+N opens no other window.

Enabling writes one preference into your own desktop-portal configuration (the file the portal reads now, or a new ~/.config/xdg-desktop-portal/<desktop>-portals.conf) and records what the file held; every other portal, such as screenshots or screen sharing, keeps its backend. If the portal service is set to read one folder only (XDG_DESKTOP_PORTAL_DIR in its environment, which some tools set to add a portal of their own), OpenXplorer works in that folder instead: it adds its line to the file the portal reads there, or a new file holding only that line, and links its packaged .portal file there, and Restore removes both again. Click Apply now to restart the desktop portal when it runs as a user service, or log out and back in. Saving over an existing file always asks first.

GTK 3 apps show GTK’s own dialog unless told to use the portal, so Enable also adds ~/.config/environment.d/60-openxplorer-file-dialogs.conf, which sets GTK_USE_PORTAL=1 for your session when you log in. Only a desktop session the systemd user manager starts (GNOME, and KDE Plasma by default) reads that file, so OpenXplorer adds it only there, while graphical-session.target is active; on other desktops, such as Xfce or MATE, GTK 3 apps keep GTK’s dialog. GTK 3 apps follow after you log out and back in; Restore removes the file again. If you enabled the dialogs with an earlier version, the status line says Click Enable for GTK 3 apps too; Enable, or Make OpenXplorer default, adds the file. If a file of yours already has that name, OpenXplorer leaves it alone and the status line says GTK 3 apps keep GTK’s dialog.

On KDE Plasma, KDE’s own apps (Plasma and its widgets, Kate, System Settings and the rest) show their own dialog unless told to use the portal, so Enable also adds ~/.config/plasma-workspace/env/openxplorer-file-dialogs.sh, which sets PLASMA_INTEGRATION_USE_PORTAL=1 when you log in. KDE apps follow after you log out and back in; Restore removes the file again. If a file of yours already has that name, OpenXplorer leaves it alone and the status line says KDE apps keep KDE’s dialog.

Restore Open and Save dialogs puts the file back as it was, or, if you edited it since, removes only OpenXplorer’s line. The Flatpak cannot change the host’s portal, so this is available in the installed package only.

```sh
cat ~/.config/xdg-desktop-portal/*-portals.conf
cat ~/.config/environment.d/60-openxplorer-file-dialogs.conf
systemctl --user try-restart xdg-desktop-portal.service
```

## Super+E on KDE Plasma

On KDE Plasma, turn on Settings → Default apps → Keyboard shortcut → Super+E opens OpenXplorer to make Super+E open a new OpenXplorer window, as Win+E opens a new File Explorer window in Windows, whether or not one is already open. Plasma gives Super+E to Dolphin; OpenXplorer asks KDE's shortcut service to move it, keeps Dolphin's other shortcuts, and records which app had it in its settings folder. The change shows in System Settings → Shortcuts and lasts across logins.

Log out and back in after turning it on or off: Plasma follows the new Super+E only from the next login. Turning it off gives Super+E back to the app that had it; if you have given Super+E to another app since, it is left alone. Other desktops and the Flatpak do not offer this.

## What this does not replace

Browsers can route reveal actions through desktop portals or keep a previous application choice. Choose OpenXplorer in a chooser when available. Upload/save file-picker dialogs stay with the system unless you turn on Open and Save dialogs, alone or with Make OpenXplorer default. On other desktops Super+E stays a separate desktop keyboard shortcut; the app changes it only on KDE Plasma, when you turn on Super+E (above).

## Tabs, windows and the taskbar

Drag a tab onto another OpenXplorer window’s tab strip to merge it, or drop it outside to create a window. You can also right-click the tab and choose Move tab to window… to select an existing destination. The original is kept until the destination accepts the tab. Close this tab’s dialog and finish file operations first.

The installed launcher offers New window, Open windows and Settings. File dragging uses its own native GTK transport: drop selected files into compatible applications, or into an OpenXplorer folder to confirm a copy. Ctrl+C/Ctrl+X/Ctrl+V remain available, including GNOME and KDE file clipboards. Native Wayland and individual application behavior need confirmation on the target desktop.

## Restore the previous setup

Disable Show in folder removes only OpenXplorer’s unmodified user-level service files. Restore previous also restores recorded file associations and gives Open and Save dialogs back to the desktop. Modified or third-party configuration is preserved for manual review. Restore Open and Save dialogs gives file dialogs back to the desktop the same way.

---

OpenXplorer 2.0.10. Project-authored documentation: AGPL-3.0-only.
