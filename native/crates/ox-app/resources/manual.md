<!-- SPDX-License-Identifier: AGPL-3.0-only -->
<!--
  The OpenXplorer user manual that F1 and More options > Help open
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

The full list of keys is in More options > Keyboard shortcuts (Ctrl+?).

## Files and folders

- New in the command bar creates a folder, a document from the built-in starters, or a file from your Templates folder.
- Cut, Copy and Paste work with other file managers. Paste on one selected folder pastes into that folder.
- Rename with F2. Select several items to rename them together.
- Delete moves items to the Recycle Bin; Shift+Delete deletes them permanently after asking. Locations without a Recycle Bin offer a confirmed permanent delete.
- When a name is already taken, OpenXplorer asks whether to replace, skip or keep both. Nothing is replaced without your answer.
- Undo (Ctrl+Z) and Redo (Ctrl+Y) reverse the last operations.
- Copies and moves show processed bytes and speed; a total and estimated time remaining appear when the total is known. Pause, Resume and Cancel apply to each job; finished items stay finished.
- Up to four copy, move or delete jobs can run while you browse. Wait for a job using overlapping source or destination folders to finish before starting another. Archive operations, renaming, restore and Undo remain exclusive.

## Views and sorting

- View switches between Details, Compact list and icon sizes; its slider and Ctrl+mouse wheel adjust the size.
- F3 toggles two panes in one tab. Click the pane you want to work in; the address bar and commands follow it.
- View can show a folder tree, enable previews and group items. Folder views can keep their own layout and sorting, or share the default selected in Settings.
- Sort offers name, modified and created dates, type, extension, size and further metadata. In Details, click a column heading to sort; the heading menu chooses visible columns, and headings can be reordered.
- In Details, expand a folder to browse its contents in place. Preview settings control network files and the maximum file size.
- Right-click empty space in a folder for View, Sort by, New, Paste and the folder's other commands.
- Ctrl+H shows or hides hidden files. Alt+Shift+P shows the details pane, F9 the navigation pane.
- Ctrl+plus, Ctrl+minus and Ctrl+0 change the text size.

## Search

- Ctrl+F moves to the search box. Typing there filters the folder you are in.
- Folders you choose can be cached for search: right-click a folder and choose Cache this folder for search. Only names and paths are stored, on this computer; no file contents are read.
- Open file location (on a result's menu) opens the folder that holds it, with the result selected.

## Network locations

- Open Network in the navigation pane, or type an address such as smb://server/share, to browse a share.
- Map network location in More options saves a share in the navigation pane.
- OpenXplorer asks for your user name and password when a server needs them, and can keep the password in your desktop's keyring.
- Right-click a share or server and choose Sign out of server to disconnect and forget a session.

## Drives and phones

- This PC lists drives, USB sticks and connected phones. Click one to open it; it is mounted when needed.
- Use Eject or Safely remove on a drive's menu before unplugging it.

## Recycle Bin

- Deleted items go to the Recycle Bin of their drive. Restore puts them back where they were.
- Empty Recycle Bin deletes everything in it permanently, after asking.

## Previous versions

- Properties > Previous versions lists snapshots that your file system or server already exposes. Browse opens one read-only; Restore a copy writes a separate copy and never replaces the original.

## Administrator access

- Choose Open as administrator on a local folder’s menu, confirm, then retry the operation in that folder. You can also enter an admin:/// address explicitly. GVfs requests desktop authentication through polkit; OpenXplorer itself stays unprivileged. This requires your distribution’s GVfs administrator backend.

## Service menus and scripts

- Choose Configure service actions from a folder or item menu. Enable each installed action you trust, then Save. Matching enabled actions appear under Service actions; uncheck them to disable them. Changed definitions must be enabled again.
- KDE service menus are read from kio/servicemenus and kservices5/ServiceMenus in XDG data folders. Executable Nautilus scripts are read from your XDG data folder’s nautilus/scripts directory. Installing or downloading scripts is your choice; OpenXplorer does neither automatically.
- Selected paths are passed as separate arguments. Terminal launchers, interpreter commands and embedded field substitutions are unsupported. Scripts receive the selected paths and URIs and current-folder URI through Nautilus environment variables.

## Settings and privacy

- Ctrl+, opens Settings, which has its own search box.
- OpenXplorer uses the network only for locations you open, server discovery you start and the update check you ask for. It has no telemetry.
- Recent files and Recent locations follow your desktop’s history privacy settings. Clear recent locations on its sidebar menu removes folder history while keeping recent files.

## Getting help

- Report an issue (in this Help window and in About this build) opens the project's issue tracker in your browser. Please describe what you did, what you expected and what happened, and leave out private file names.
