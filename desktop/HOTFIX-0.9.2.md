# OpenXplorer 0.9.2 — remove the extra top menu row

This is a focused native-window hotfix. It retains the 0.9.1 features,
mobile documentation behavior and fictional public examples.

## What caused the extra row

The host registers a GTK application menu for New window, Open windows,
Settings and Quit. Gtk.ApplicationWindow defaults to showing that menu inside
the window when the desktop does not export it. With our existing HTML tab
strip, this produced a redundant row labelled OpenXplorer above the tabs.
This is a native GTK menu, not website/HTML chrome.

## The change

Every window now calls `self.window.set_show_menubar(False)` immediately after
constructing `Gtk.ApplicationWindow`, before it is mapped. The shared creation
path covers normal windows, Settings windows and detached tabs.

The application name, registered actions, desktop identity, custom zero-height
client-side titlebar, window buttons, drag handlers and resizing are retained.
There is no `set_decorated(False)` workaround and no global GTK/theme change.

## Install

Finish file operations first. Quit the existing application and optional
background reveal service so the update does not reconnect to the old process:

```sh
openxplorer --quit
sudo apt install ./openxplorer_0.9.2_all.deb
openxplorer
```

If quitting reports active file operations, let them finish and retry. Do not
force-kill a transfer. Closing one window is not necessarily quitting the app.
The running status bar should show 0.9.2. Settings, pins, stored credentials,
search indexes, defaults and mounts are not reset or migrated by this fix.

## Validation boundary

A real GTK 3 test on a virtual X11 display reproduces a 27-pixel fallback menu
in the baseline and no menu with the production policy. It checks a second
window, repeated show-all, shell-export setting changes, retained actions,
title metadata and resize/decorations flags.

This is a native GTK window-policy harness, not a complete OpenXplorer/WebKit
launch. Zorin, Wayland, live NAS access and the Next.js production build are not
validated here. See TEST-REPORT.md for the executed checks.

## Reference

GTK 3 documents the behavior of `Gtk.ApplicationWindow:show-menubar`:
https://docs.gtk.org/gtk3/property.ApplicationWindow.show-menubar.html
