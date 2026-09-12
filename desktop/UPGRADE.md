# OpenXplorer 1.0.0-rc.4 — tabs and desktop routing

This release fixes missing tab reattachment and distinguishes ZIP opening from
folder/SMB defaults and browser file-reveal integration. It remains a release
candidate. Changes to desktop settings are explicit and reversible.

## Upgrade

Finish active file operations first. Install the new package, then restart the
old background process using the new launcher:

```sh
sudo apt install './openxplorer_1.0.0~rc4_all.deb'
openxplorer --restart
```

Settings, pins and credentials keep their existing compatibility paths. The
fallback GTK app-menu remains disabled. The reproducible package timestamp is
new for this build so Python can invalidate timestamp-based bytecode on upgrade.

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
