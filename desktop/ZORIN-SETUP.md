# OpenXplorer 0.8.0 — Zorin and Brave setup

This is a development release. It targets the Zorin 18 native desktop stack.
The package is built and structurally checked; native GTK/WebKit, Zorin’s panel,
Software, a real NAS, and Brave have not been exercised in the release container.
Use disposable files and a test share before making it your everyday file manager.

## Install / update

Close every old OpenXplorer window. From the download directory:

```sh
sudo apt update
sudo apt install ./openxplorer_0.8.0_all.deb
openxplorer --check
openxplorer
```

Run OpenXplorer as your normal desktop user, never with sudo. The menu entry is
**OpenXplorer**, with a yellow folder. The status bar should say **0.8.0**. If it does
not, an old process/window is still running. Starting with 0.7, `openxplorer --quit`
closes all windows safely and ends its optional background service; it refuses
while file writes are in progress. For a subsequent upgrade, quit this way first.

For a blank native window, close all instances and use:

```sh
openxplorer --software-rendering
```

The same package is inside the source archive’s `dist/` directory. APT installs
native runtime dependencies; the source ZIP does not contain Linux system libraries.

## Make folders and browser “Show in folder” use OpenXplorer

1. Open Settings (gear or Ctrl+,) → **Default file explorer**.
2. Leave **Include Brave / other apps’ Show in folder integration** checked, then
   press **Make OpenXplorer default**. This is an explicit per-user choice, not an
   installation side effect. Already-default users should press it again to add
   the new integration, or use **Enable Show in folder**.
3. Check the separate Show in folder status. “OpenXplorer is handling FileManager1
   requests” means OpenXplorer owns the standard session-bus endpoint. “Waiting”
   means another application still owns it. Finish operations and close other
   file managers; if it remains waiting, log out of Zorin and log back in.
4. Click **Test Show in folder**. It sends a real FileManager1 ShowFolders request
   for your home directory. It does not test Brave’s separate portal path.
5. Restart Brave and use Downloads → **Show in folder** on a downloaded test file.
   With a direct ShowItems request, OpenXplorer opens the real parent and selects
   that item, including a PDF or video; it does not try to browse the file itself.

Why two defaults? `inode/directory` and `x-scheme-handler/smb` are file associations.
`org.freedesktop.FileManager1` is a different desktop API for revealing files.
Modern Chromium-based browsers can prefer a desktop portal’s OpenDirectory API,
then fall back to FileManager1 or opening the parent directory. Portal backend,
packaging and remembered choices can affect which application opens. Thus this
release does not promise that one toggle overrides every possible browser route.

If a portal offers an application chooser, select OpenXplorer there. A portal that
retains another selection may need its specific chooser preference changed.
Do **not** disable the desktop portal, remove Zorin Files, or kill file-manager
processes during file operations. Upload/Save As file-picker dialogs remain the
system’s dialogs; changing a file-manager default does not replace them.
Snap/Flatpak apps may have additional path and permission restrictions.

Useful, non-destructive checks:

```sh
xdg-mime query default inode/directory
xdg-mime query default x-scheme-handler/smb
openxplorer --windows
```

The two MIME queries should print `io.winspace.Development.desktop` after a
successful change. To reveal a specific existing file independently of Brave:

```sh
openxplorer --select "/absolute/path/to/an/existing/test.pdf"
```

The optional integration creates ONLY these user files (respecting XDG overrides):

```
~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service
~/.config/autostart/io.winspace.FileManager1.desktop
~/.config/winspace/reveal-integration.json
```

The service stays available with no window so another application can request
file reveal. The autostart launches that service without opening a window at
login. It does not run searches or mount shares just by starting. Existing unknown
service overrides are refused instead of overwritten. **Disable Show in folder**
removes OpenXplorer’s unmodified user integration files; **Restore previous** also
restores the recorded MIME associations. Modified user files are preserved.
Disable/restore before uninstalling so there is no stale user override.

## Taskbar, existing windows and detachable tabs

Pin the installed **OpenXplorer** application, not a source script or the downloaded
installer, to the Zorin panel. If an older pin remains generic or creates a second
group, remove that pin and pin the installed application again.

Every new native window is managed by one Gtk.Application identity. Right-click
the pinned app → **Open windows…**, or use the window-list button near OpenXplorer’s
window controls. It lists existing windows, including those showing Settings.
**New window** / Ctrl+N creates another. `openxplorer --windows`, `--new-window`, and
`--settings` provide the same actions. Zorin’s panel still controls whether an
ordinary click minimizes, cycles, or shows its own window overview.

Drag a tab down out of the tab strip until “Release to move tab to a new window”
appears, then release. Right-click the tab → **Move tab to new window** is the
non-drag alternative. Its address, history, view, selection and scroll are sent to
the new window; the original is removed only after an explicit ready acknowledgment.
A failed or timed-out handoff keeps the source. Close that tab’s dialog and finish
file operations first. A compositor may position the new window itself; this is
not a guarantee that it follows the pointer across screens. Moving tabs into a
*different existing* window is not implemented in this release.

## Network entries

Successful server/share browsing now adds an entry under Network. Active GVfs SMB
mounts and recognized persistent CIFS mounts appear there as well. This is separate
from Quick access pins. A green marker means network-backed, not guaranteed online.

Right-click an unsaved share → **Keep in Network** to retain it after quitting.
Browsing alone does not persist a bookmark, save credentials, configure fstab or
change mount permissions. Unsaved visited entries belong to the application
session; an active mount can still be visible after relaunch. Saved locations
retain their chosen names and reconnect on open. Host aliases/IPs are not guessed
or merged for credentials. Sign out affects other desktop apps using that server.

## Downloads and Brave

A network download directory needs an existing stable writable Linux path backed
by a persistent mount, for example `/mnt/nas/downloads`. An SMB bookmark such as
`smb://nas/downloads` or a temporary `/run/user/.../gvfs` path is not enough.

Right-click Downloads → Properties → Location, choose/check your destination,
confirm, and Apply. This changes the Linux standard-folder setting, not existing
file contents. The additional **Also update Brave’s download directory** checkbox
starts unchecked. Checking it opens a separate profile/consent dialog after the
Linux setting has been applied. A browser-sync failure does not undo that Linux
folder change, and the two results must not be confused.

Alternatively, Settings → **Brave & downloads** → **Use Linux Downloads in Brave…**
performs the browser step independently. Fully quit Brave, including background
processes, and keep it closed until the sync is finished. Select profiles, check
the backup/update confirmation, and Apply to Brave. OpenXplorer refuses detected
running-browser writes; it never kills the browser.

This is an explicit one-time edit for detected **native** Brave profiles under
`$XDG_CONFIG_HOME/BraveSoftware/Brave-Browser*` (Default/Profile N). It updates only
`download.default_directory` and `savefile.default_directory`; it does not touch
history, passwords, extensions or unrelated preferences. It writes a private
backup and performs an atomic replacement after rechecking the original bytes.
The Preferences format is a browser implementation detail, not a stable public
Brave API; this integration still needs on-device testing. Do not launch Brave
concurrently with it. Managed browser policy may override the chosen directory.

Backups and restore records are under `~/.config/winspace/brave-backups/`, private
to your account (directory 0700, files 0600). Backups contain the whole original
Preferences JSON and can include private metadata; never attach them to a public
bug report. **Restore previous** in the browser dialog restores only unchanged
OpenXplorer-applied directory keys for the one selected profile; unrelated newer
preferences are not rolled back.

For custom profiles, Snap, Flatpak, managed browsers, or a missing detected profile,
open `brave://settings/downloads` in Brave and select the same stable Linux path
manually. A sandbox may also need access to that mount. This is not an ongoing
sync policy and does not make downloads work while the NAS is offline.

## Settings search and the input fix

Use **Search settings** in the Settings page’s left navigation. Results include
themes, context menus, cache folders, network refresh interval, defaults, windows,
and Brave. Typing scrolls to and highlights the first matching control; click
another result to highlight it. Escape clears the search. Settings values are
not changed by searching. The custom cache path input expands to the available
width and the Add button can wrap in a narrow window.

## Installer icon, name and license

No website is needed to ship an app icon. The package installs the folder icon
(SVG plus PNG sizes), a matching desktop launcher, and AppStream metainfo with
**OpenXplorer**, **MIT**, and **0.8.0** release information. An installed application’s
icon is different from Software’s generic icon for opening an arbitrary local
`.deb` before its metadata is indexed. Zorin Software may still show a generic or
cached local-package card. Close/reopen Software after installation; distribution
catalog/Software behavior has not been tested here. Do not expect a website alone
to change that. No made-up website/repository, signature, or distribution approval
is claimed by this package.

## Upstream design references

- Chromium browser reveal routes: https://raw.githubusercontent.com/chromium/chromium/main/chrome/browser/platform_util_linux.cc
- FileManager1 specification: https://www.freedesktop.org/wiki/Specifications/file-manager-interface/
- GtkApplication windows/actions: https://docs.gtk.org/gtk3/class.Application.html
- AppStream desktop application metadata: https://www.freedesktop.org/software/appstream/docs/sect-Metadata-Application.html
- Chromium download preference implementation: https://raw.githubusercontent.com/chromium/chromium/main/chrome/browser/download/download_prefs.cc

Source-reviewed on 2026-09-06. Reading upstream code does not constitute native
integration validation of this build.
