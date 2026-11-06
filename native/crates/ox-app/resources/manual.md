<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!--
  The OpenXplorer user manual that F1 and Settings > About > Help open
  (CMD-033). It is compiled into the program, so it is installed with
  every package and works offline. Each "## " heading starts a topic of
  the Help dialog; src/window/help.rs splits it there.
-->

## Getting started

OpenXplorer shows your folders, drives, phones and network shares in one tabbed window.

- The navigation pane on the left lists Quick access, This PC, Network and the Recycle Bin. Choose Pin to Quick access on a folder's menu to keep it there.
- The address bar shows where you are. Click a part of the path to go there, or click the empty space beside it (or press Ctrl+L) to type an address such as a folder path or smb://server/share.
- Back, Forward and Up are beside the address bar (Alt+Left, Alt+Right, Alt+Up). Press F5 to refresh.
- Ctrl+T opens a tab, Ctrl+W closes it and Ctrl+Shift+T reopens the last closed tab. Ctrl+N opens a new window. Drag a tab out of the tab strip to give it a window of its own.
- Type the start of a name to select it in the folder.

The full list of keys is in Settings > About > Keyboard shortcuts (Ctrl+?).

## Files and folders

- New in the command bar lists what Windows 11 lists: Folder, Shortcut, Bitmap image, Rich Text Document, Text Document and Compressed (zipped) Folder, then the files in your Templates folder. A new item is made at once as, for example, "New Text Document.txt" (or "New Text Document (2).txt" when that name is taken), then renamed in place: type its name and press Enter.
- Cut, Copy and Paste work with other file managers. Paste on one selected folder pastes into that folder.
- Rename with F2. Select several items to rename them together. As in Windows, \ / : * ? " < > | can’t be typed in a name, and a file given a name that is taken is offered the next free number, such as Report (2).txt.
- Delete or Ctrl+D moves items to the Recycle Bin; Shift+Delete deletes them permanently after asking. Locations without a Recycle Bin offer a confirmed permanent delete.
- When a name is already taken, OpenXplorer asks whether to replace, skip or keep both. Nothing is replaced without your answer.
- Undo (Ctrl+Z) and Redo (Ctrl+Y) reverse the last operations.
- Copies and moves show processed bytes and speed; a total and estimated time remaining appear when the total is known. Pause, Resume and Cancel apply to each job; finished items stay finished.
- Up to four copy, move or delete jobs can run while you browse. Wait for a job using overlapping source or destination folders to finish before starting another. Archive operations, renaming, restore and Undo remain exclusive.

## Views and sorting

- View switches between Details, Compact list and icon sizes; its slider and Ctrl+mouse wheel adjust the size.
- View > Split view toggles two panes in one tab. Click the pane you want to work in; the address bar and commands follow it.
- View can show a folder tree, enable previews and group items. Folder views can keep their own layout and sorting, or share the default selected in Settings.
- Sort offers name, modified and created dates, type, extension, size and further metadata. In Details, click a column heading to sort; the heading menu chooses visible columns, and headings can be reordered.
- In Details, expand a folder to browse its contents in place. Preview settings control network files and the maximum file size.
- Right-click empty space in a folder for View, Sort by, New, Paste and the folder's other commands.
- Ctrl+H, or View > Show > Hidden items, shows or hides hidden files. Alt+Shift+P shows the details pane and Alt+P the preview pane, which take turns in one place; F9 shows the navigation pane.
- As in Windows, the extensions of known file types are hidden: Notes.txt shows as Notes. View > Show > File name extensions shows them. Renaming keeps the extension.
- F11 shows the window full screen; press it again to get the window back as it was.
- Ctrl+plus, Ctrl+minus and Ctrl+0 change the text size.

## Search

- Ctrl+F, Ctrl+E or F3 moves to the search box. Typing there filters the folder you are in.
- Folders you choose can be cached for search: right-click a folder and choose Cache this folder for search, or use Cache this folder for search in Settings > Search for the folder you opened Settings from. Only names and paths are stored, on this computer; no file contents are read.
- Open file location (on a result's menu) opens the folder that holds it, with the result selected.
- A search is a step of the tab's history, as in Windows: the tab and the address bar read Search Results in Documents, Back returns to the folder and Forward to the results, and Back from a folder you opened from the results shows them again. Emptying the search box goes back to the folder too.

## Network locations

- Open Network in the navigation pane to see the computers and servers on your network, as in Windows; open one to see its shared folders. Refresh (F5) looks again. Or type an address such as \\server\share or smb://server/share in the address bar.
- Network finds only devices that advertise themselves, through the desktop's GVfs network services (DNS-SD through Avahi, Windows computers through WS-Discovery); firewalls and separate networks can hide them. If Network says that network discovery is turned off, those services are missing: install your distribution's GVfs backends (gvfs-backends on Ubuntu and Debian) and Avahi, or type the server's address instead.
- Map network drive, on the right-click menu of This PC and Network in the navigation pane or in their "…" menu, connects a share such as \\server\share under a drive letter, as in Windows; Reconnect at sign-in saves it in the navigation pane. The letter is only part of its name. Its link Connect to a Web site, and Add a network location in that menu, connect SFTP, FTP, WebDAV and NFS folders too. Disconnect network drive there disconnects a connected share.
- OpenXplorer asks for your user name and password in Enter network credentials when a server needs them, and can keep the password in your desktop's keyring. Connect using different credentials in Map network drive asks even when it knows an account.
- Right-click a share or server and choose Sign out of server to disconnect and forget a session.

## Drives and phones

- This PC lists drives, USB sticks and connected phones. Click one to open it; it is mounted when needed.
- Use Eject or Safely remove on a drive's menu before unplugging it.

## Recycle Bin

- Deleted items go to the Recycle Bin of their drive. Restore puts them back where they were.
- Empty Recycle Bin deletes everything in it permanently, after asking.

## Previous versions

- Properties > Previous Versions lists snapshots that your file system or server already exposes. Select one: Open shows it read-only in a new tab; Restore writes a separate copy and never replaces the original.

## Properties

- Properties opens in a window of its own, as in Windows 11, with OK, Cancel and Apply. Changes to the name, Read-only, Hidden and a folder's icon wait for OK or Apply; Cancel discards them.

## Administrator access

- Choose Open as administrator on a local folder’s menu, confirm, then retry the operation in that folder. You can also enter an admin:/// address explicitly. GVfs requests desktop authentication through polkit; OpenXplorer itself stays unprivileged. This requires your distribution’s GVfs administrator backend.

## Service menus and scripts

- Choose Configure service actions from a folder or item menu. Enable each installed action you trust, then Save. Matching enabled actions appear under Service actions; uncheck them to disable them. Changed definitions must be enabled again.
- KDE service menus are read from kio/servicemenus and kservices5/ServiceMenus in XDG data folders. Executable Nautilus scripts are read from your XDG data folder’s nautilus/scripts directory. Installing or downloading scripts is your choice; OpenXplorer does neither automatically.
- Selected paths are passed as separate arguments. Terminal launchers, interpreter commands and embedded field substitutions are unsupported. Scripts receive the selected paths and URIs and current-folder URI through Nautilus environment variables.

## Settings and privacy

- … > Options opens Folder Options, as in Windows 11. General: where new windows open, whether each folder you open from the file list opens in its own window, whether the navigation pane shows Recent files and Recent locations, and Clear File Explorer history. View: Apply to Folders, Reset Folders and the Advanced settings, such as hidden files, file name extensions, the status bar, pop-up descriptions and typing into the search box. Search: Always search file names and contents. OK or Apply saves the choices; Clear acts at once.
- Ctrl+, or the gear in the command bar opens Settings, which has its own search box; so does More settings… in Folder Options (… > Options). Settings > Appearance chooses Light, Dark or the system appearance, Settings > Default apps makes OpenXplorer the default file explorer when you ask it to, and Settings > About has the keyboard shortcuts, this manual, the licence and About this build.
- OpenXplorer uses the network only for locations you open, server discovery you start and update checks: the one you ask for in Settings › About, and one a day when the app starts, which Check for updates automatically turns off. It has no telemetry.
- When a newer release is out, a notice pops up at the top right, Update shows in the command bar and Update available at the bottom right. Later closes the notice for that release; nothing installs until you choose Install update.
- Recent files and Recent locations follow your desktop’s history privacy settings. Clear recent locations on its sidebar menu removes folder history while keeping recent files.

## Getting help

- Report an issue (in this Help window and in About this build) opens the project's issue tracker in your browser. Please describe what you did, what you expected and what happened, and leave out private file names.
