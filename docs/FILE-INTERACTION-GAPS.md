# File interaction gap analysis

Reviewed 2026-09-11 against the desktop implementation in this source tree. The
request was to make dragging files into other applications, including T3 Code,
work like a traditional file explorer. This is a code audit and implementation
record, not a claim that every desktop, compositor or destination application
has been tested. Source line numbers below are review-time locations; function
names identify the code if later edits shift those lines.

## Root cause

The original file list did not initiate an operating-system file drag. Its
`renderRows` attached `makePinDraggable` only to directories, and
`pointerPinMove` restricted the selection to folders. That gesture only moved an
in-page badge and pinned a shortcut in Quick access. Ordinary files and mixed
selections could not leave the window. The existing native drag transport in
`desktop/native_tab_drag.py:61` offered an application-private tab target and
an empty desktop handshake, rather than a file list. HTML drop handlers also
explicitly rejected external transfers. Adding editor-specific text to the
existing pointer badge would not repair that missing native transport.

T3 Code provides an example of why native files matter: its workspace drop
handler tests for the browser's `Files` type, sets copy feedback, and reads
`DataTransfer.files`. Plain path text alone does not satisfy that handler.
This finding is from source inspection of commit
`4a4c6dd2adc350a68ba18bb28b24b5a7e4660dab`, not a test of the user's installed
version. Individual T3 Code surfaces can have different attachment or folder
policies. [T3 Code workspace file drop source, lines 19–51](https://github.com/pingdotgg/t3code/blob/4a4c6dd2adc350a68ba18bb28b24b5a7e4660dab/apps/web/src/components/chat/workspaceFileDrop.ts#L19)

GTK supplies native URI targets and `SelectionData.set_uris`; GTK 3.24.37 and
later can also use the FileTransfer portal when supplying these targets. The
implementation now uses these APIs. Whether a particular sandboxed receiver
can read a file still needs an application/session integration test.
[GTK URI selection API](https://docs.gtk.org/gtk3/method.SelectionData.set_uris.html)

## Prioritized findings and current disposition

“Implemented” means the change exists in this working tree. Validation evidence
and remaining real-desktop checks are listed separately below.

| Priority | Interaction gap | Current disposition and code evidence |
| --- | --- | --- |
| P0 | Drag ordinary files into editors or attachment fields. The original pointer gesture never exported files to the desktop. | **Implemented:** a real GDK gesture starts a GTK drag; URI targets export local file URIs and text targets offer readable paths. `desktop/native_file_drag.py:163` (`begin`) and `:190` (`data_get`). Sources are copy-only; no source deletion occurs in `data_delete` at `:200`. |
| P1 | Drag a multi-selection, including both files and folders, without collapsing it to the item under the pointer. | **Implemented:** `beginNativeFileDrag` in `desktop/ui/app.js` uses the complete selection when dragging a selected entry, or selects the unselected entry being dragged. `prepare_files` in `desktop/native_file_drag.py:39` validates and deduplicates up to 200 items. Ctrl/Shift selection remains in `desktop/ui/app.js:366` (`selectEntry`). |
| P1 | Drop files from another file manager into an open directory or a folder row. Original DOM handlers rejected external drops. | **Implemented:** `desktop/native_file_drop.py:50` decodes bounded, explicit file/SMB URI lists; `:125` validates the received destination. `desktop/ui/app.js:504` publishes folder and background targets; `:577` (`receiveFileDrop`) asks for Skip duplicates or Keep both and calls the existing transfer engine. Dragging does not implicitly overwrite a file. |
| P1 | Copy files by dragging between OpenXplorer windows or into a subfolder. Previously only Quick access pinning was possible. | **Implemented for copy:** uses the same native source/receiver and existing transfer engine. Same-process drops retain canonical SMB URIs even if the external representation uses an existing local mount (`desktop/native_file_drop.py`, `received`). Quick access remains a pin/reorder target, not a file-copy destination. |
| P1 | Clipboard cut from KDE is interpreted as copy. | **Fixed:** export already supplied `x-kde-cutselection`, but reads ignored it. `desktop/file_clipboard.py:126` now requests that marker only alongside a valid URI-list fallback. Only the exact cut marker gives move semantics; plain text does not become a file list. An owner-change guard rejects mismatched asynchronous replies. |
| P1 | Completed cuts from an external GNOME clipboard cannot be consumed reliably. | **Fixed:** external formats previously received a new random token on every read, so the completion token never matched. `desktop/file_clipboard.py:45` now derives a stable token from the external payload and mode; `:151` consumes only completed items from the matching current clipboard. Changed payloads and changed owners are preserved. |
| P1 | Remote files may be browseable here but unusable by an editor that requires a local path. | **Partially addressed:** `desktop/native_file_drag.py:39` reuses `desktop/native_opening.py:19` (`local_path`) to resolve already-mounted CIFS/GVfs paths. Without one, an SMB URI is offered; a file-only receiver may reject it. The drag does not mount, authenticate or download files. |
| P2 | Traditional drag-to-move and Ctrl/Shift/Alt action negotiation are absent. | **Deferred:** drag sources and receivers intentionally use copy. Cut/Paste remains the explicit move operation. Reliable destructive drag negotiation, including destination failure and cancellation, needs a separate design and native tests. |
| P2 | Cross-filesystem cut/move is absent. | **Existing explicit limitation:** `desktop/operations.py:145` delegates only to native moves; `desktop/gio_backend.py:19` uses `NO_FALLBACK_FOR_MOVE`. Do not implement this by naively copying and then deleting the source. A future implementation needs durable completion, metadata/error policy and cancellation recovery. |
| P2 | ZIP members and virtual network objects are not ordinary OS-backed files. | **Existing boundary:** `fileDragEntry` in `desktop/ui/app.js:477` excludes archive and virtual entries; native source validation rejects unsupported schemes (`desktop/native_file_drag.py:24`). Concrete share-root references can be dragged out or pinned, but copying a whole SMB share remains rejected by the receiver/operation checks (`desktop/native_file_drop.py`, `received`; `desktop/core.py:363`). Extract ZIP contents first, and open a share to select its children when copying. |
| P2 | No file-operation undo or richer duplicate handling. | **Deferred:** `desktop/operations.py:92` only accepts Skip duplicates or Keep both. There is no overwrite/merge or undo journal. New drop handling deliberately reuses those existing semantics. |
| P2 | Some keyboard and selection conveniences still trail a full explorer. | **Deferred:** icon-view Up/Down currently move by one item instead of by the number of visible columns (`desktop/ui/app.js`, `onKey`); there is no file-list Left/Right or rubber-band selection. `copyPath` at `:423` handles a single selected path or the current directory, not all selected paths. These are useful follow-ups independent of the native drag fix. |

## Behaviors to preserve

The transfer engine already rejects copies into the source folder or one of its
descendants, including local symlink aliases (`desktop/operations.py:67`). Copies
use private staging and a non-overwriting final rename (`TransferEngine.run`);
the drag receiver must continue through that engine. Snapshot destinations retain
their read-only checks. The source-side copy action prevents OpenXplorer from
deleting dragged files; it does not make the original file read-only to an editor
that opens it.

Tabs retain their separate private transport, acknowledged handoff and Escape
behavior. A file drag must not detach a tab. File-manager defaults, browser
profiles, privileged mounts and folder relocation are not prerequisites for
drag-and-drop and remain opt-in.

The browser preview contains fictional data and has no access to real desktop
files. Its existing pin simulation is not native drag validation.

## Validation evidence and remaining checks

The clipboard changes were tested with this command:

```sh
python3 -m unittest desktop.tests.test_file_clipboard_interop desktop.tests.test_v05
```

Result: **100 tests passed**, including 13 new protocol/async regressions. They
cover repeated external-cut reads, partial and complete consume, changed payloads,
KDE cut/copy marker handling, format priority, text/unsafe-URI rejection and owner
changes between asynchronous replies. These tests use queued selection doubles;
they do not claim native GTK clipboard or actual KDE/GNOME session validation.

The [current test report](../desktop/TEST-REPORT.md) records 542 Python tests,
30 file-drag browser checks, 27 native file-transport checks and 24 native tab
checks. The native file test uses a real WebKit source, a GTK URI receiver, and
a separate headed Chromium process that received readable `DataTransfer.Files`.
It uses synthetic HTML/controllers and temporary local files on isolated X11;
it does not run the full application UI, installed T3 Code, Wayland or SMB.
The browser UI contract separately exercises the shared application JavaScript.
A same-process WebKit receiver returned an empty browser file list; compatibility
with that receiver remains unverified. The following are acceptance checks still required wherever that
report does not explicitly record them as run:

1. In an actual GTK/WebKit session, drag `/home/demo/Read me.txt`, a folder and a
   mixed selection into a native URI receiver, a file-manager window and the
   intended T3 Code surface. Check that file receivers see files, not only text.
   Include filenames containing spaces, Unicode, `#`, `%` and `?`.
2. Repeat the source/destination test on both supported Wayland and X11 sessions,
   with the installed editor packaging (including Flatpak/Snap if relevant).
   Distinguish desktop/portal restrictions from a missing URI payload.
3. Drop files into the current directory, a visible subfolder and another
   OpenXplorer window. Verify target highlighting, destination confirmation,
   duplicate handling and cancel. Confirm Quick access pins folders and never
   copies content into a pinned destination implicitly.
4. Cancel or reject the drag, close a menu/dialog, scroll the virtualized list,
   change text size and switch between details/icons. Check selection retention,
   coordinate alignment, cleanup and absence of accidental double-click opens.
   Run existing tab drag tests to catch transport interference.
5. Test already-mounted SMB items using both local mount paths and canonical SMB
   locations, then a remote item without a local export. Confirm the latter's
   limitation is visible and no connection or mount setup occurs during drag.
   Test protected snapshot destinations and ZIP-member rejection separately.
6. In real GNOME and KDE file managers, cut two disposable files, paste one
   successfully and let one fail. Confirm only the completed item is consumed.
   Copy unrelated text during an operation and confirm that text is preserved.

Use only the fictional fixtures required by [the privacy rules](PRIVACY.md).
Run the public-data audit after any captures and after staging the complete
release. Chromium simulations and pure Python checks are not native WebKit,
SMB, portal or T3 Code end-to-end tests.

Project-authored documentation: AGPL-3.0-only.
