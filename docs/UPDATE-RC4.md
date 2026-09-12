# OpenXplorer 1.0.0-rc.4 — mouse and tab navigation

## Update

Finish file operations first, then install and restart the existing application process:

```sh
sudo apt install './openxplorer_1.0.0~rc4_all.deb'
openxplorer --restart
openxplorer --diagnose
```

Closing a window alone may leave the optional background service running. The restart command requests a safe shutdown; it does not force-kill active file operations. Preferences, pins, credentials and file associations are not reset.

## Middle-click to open a tab

Middle-click a folder or navigable location to create a **background tab**. Shift+middle-click creates and switches to the tab. Middle-click an existing tab to close it. Details and icon views, cached folder results, breadcrumbs, sidebar pins, connected shares, discovered servers and mounted drive cards all support this action. Mount an unmounted volume with an ordinary click first.

A background tab does not list its directory or request network credentials until selected. Its address is the real folder location, not a search-results URL. Regular files are not converted into directory tabs by their extension; a directory called `Archive.mp4` still works. ZIP files keep their normal archive-opening behavior.

The action uses mouse down/up events for compatibility with WebKit versions without `auxclick`. A subsequent auxiliary click is suppressed, so one gesture does not create two tabs. Middle-click elsewhere, including text inputs, is unchanged.

## Tear out and merge tabs

Drag a tab onto another OpenXplorer window's tab strip to merge it. For a new window, drop on the desktop where the compositor supports tab tear-out, or pull the tab down into its own window's file area until **Release to open this tab in a new window** appears. Leave a small gap below the tab strip before releasing. The existing **Move tab to new window** menu remains an alternative.

The native source now offers GTK's `application/x-rootwindow-drop` target as an **empty signal**, alongside the process-local merge token. The compositor never receives a file path, credential or merge capability through the desktop target. New-window creation waits until the drag finishes and the pointer grab is released. Escape and generic drag errors keep the source tab. Dropping into another window's file area is not a merge or tear-out target.

The source tab is retired only after the existing handoff acknowledges that the destination is ready. Busy windows, failed startup and timeouts retain the original. This change concerns **tabs**, not dragging file icons into other applications.

## Why a fresh local package card is still generic

The installed folder icon, AGPL-3.0-only license, launcher identity, package mapping and project URL are retained. The installed card is the one that can use that catalog.

Opening a freshly downloaded `.deb` before installation is a separate Software path: the package's catalog has not yet been installed/indexed, and a locally imported package may be shown as a generic package rather than a matched desktop application. Another embedded icon or website URL alone cannot guarantee a fully populated pre-install card.

For a first-install catalog experience, distribute through a signed APT repository with AppStream metadata that is available to Software **before** the application is installed. That requires publishing and configuring a real repository/catalog. This release does not deploy one, install a signing key, change a system repository, or run a privileged metadata bootstrap merely to decorate an installer. Software versions and cache state may also differ.

## Evidence and limits

See the release test report. Native GTK drag tests use real X11 pointer gestures with a compositor-protocol test receiver, not Mutter/Wayland or a complete WebKit window. Chromium mouse tests exercise the shared application HTML with simulated files. Native Zorin/Wayland behavior, live SMB, Software's clean pre-install view and the Next.js production build remain target-environment checks.

## Technical references

- GTK notebook's desktop-drop handshake: https://raw.githubusercontent.com/GNOME/gtk/gtk-3-24/gtk/gtknotebook.c
- GNOME compositor support for root-window drops: https://bugzilla.gnome.org/show_bug.cgi?id=762104
- AppStream catalog distribution: https://www.freedesktop.org/software/appstream/docs/chap-CatalogData.html
