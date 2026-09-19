# OpenXplorer 1.1.1 — immediate startup layout

The native app now displays its window layout immediately instead of the
centered startup message and spinner. Static placeholders stay visible until
the real interface is ready. Error recovery and software-rendering retry remain.

From 1.1.0, use **Check for updates** in the sidebar.

## Upgrade once from an older release

Finish active file operations and quit OpenXplorer, then install the package:

```sh
openxplorer --quit
sudo apt install ./openxplorer_1.1.1_all.deb
openxplorer --restart
```

## Later updates inside the app

Click **Check for updates** in the sidebar. Review the version and release notes,
then click **Install update**. The app verifies the GitHub release asset digest
and package identity before invoking APT with a system administrator prompt.
Finish transfers, folder loading and tab moves before installing. File actions
are blocked during installation and until restart. **Restart now** closes the
existing windows and tabs and starts the installed version.

Checks and installations are opt-in. Source checkouts can check releases but
cannot install through the app. This is a native package update, not live code
replacement. GitHub HTTPS and asset digests provide integrity; releases are not
independently signed. Failed downloads leave the installed app unchanged. If
APT reports a configuration error, repair the system package state before retrying.

Settings, pins and credentials keep their existing compatibility paths.

## Merge a tab back into a window

Drag the tab label onto another OpenXplorer window’s tabs or the blank space
beside them. An insertion highlight marks the destination. Drop to insert the
tab; dragging outside creates a new window. Escape cancels. Same-window drops
reorder tabs. This uses a native GTK drag, not an HTML pointer constrained to the
source WebView.

Alternatively, right-click the tab → **Move tab to window…** and choose an
existing window. Source and destination must belong to the same OpenXplorer
application process. Close that tab’s dialogs and finish file operations first.
If the destination is busy, disappears, rejects the move, or times out, the source
is kept. Moving the last tab closes its now-empty source window after acceptance.

Tabs transfer whitelisted navigation state, not credentials or file content.
Native dragging uses a short-lived random, single-use capability restricted to
this application. It never exports a file URI or initiates a filesystem move.
File-icon dragging now uses a separate native GTK file transport. Select files
or folders and drag them into a compatible application, or drop into an
OpenXplorer folder to confirm a copy. Quick access drops still pin folders.
The shared Ctrl+C/Ctrl+X/Ctrl+V file clipboard remains available.

File drops are copy-only. Extract ZIP contents first. Editors that require local
files need an already-mounted GVfs/FUSE or CIFS path for network items; dragging
does not mount shares or download a temporary copy.

## ZIP files from Brave

Clicking a download’s ZIP filename asks the desktop to open a ZIP. That is not
**Show in folder**. The earlier default control set only folder and SMB handlers.

Settings → Default file explorer now lists **Folders**, **SMB links**, **ZIP
files**, and FileManager1 ownership separately. Choose **Use OpenXplorer for
ZIPs** to browse ZIP downloads in OpenXplorer. Use **Restore ZIP handler** to
return to the recorded archive application without changing your folder defaults.
The new ZIP option is not silently enabled during installation or upgrade.

A ZIP opened this way is browsed, not automatically extracted. PDF and video
associations stay unchanged.

## Brave’s Show in folder

Enable Show in folder integration and review the actual owner. “Enabled” or a
folder MIME default is not proof that OpenXplorer owns FileManager1. A competing
app can refuse to release the service. Finish operations and close its windows;
log out and back in if necessary. No other process is force-killed.

Restart Brave after changing settings. The built-in test checks FileManager1,
not the browser’s portal route. A remembered portal choice can still select
another application; select OpenXplorer in a chooser when available. Do not
disable the desktop portal or assume upload/save file pickers are replaced.

Read-only diagnostics:

```sh
xdg-mime query default inode/directory
xdg-mime query default application/zip
openxplorer --diagnose
```

The diagnostic report now includes individual MIME handlers and the current
FileManager1 owner process name. It does not list files or credentials. A
`.desktop` ID of `io.winspace.Development.desktop` is the retained compatibility
identifier for OpenXplorer, not an obsolete installation.

## Test boundary

See TEST-REPORT.md. Native GTK tab transport is tested with two GTK surfaces under
Xvfb through a ctypes adapter, while the exact UI/dispatcher transfer contract is
tested with two Chromium pages. Neither test is a full WebKit/Wayland/Zorin/Brave
session. Native SMB and the Next.js production build remain unverified here.
