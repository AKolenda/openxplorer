# Native desktop experience roadmap

Reviewed 2026-09-11 against the desktop sources after the file drag work. This
document recommends follow-up work; it does not authorize every proposed
feature or claim that it has been implemented. Line references are review-time
locations, with function names supplied where later edits may shift them.

The next improvement should be reliable everyday interactions: keyboard
selection, useful focus feedback, fresh directory listings and understandable
operations. Thumbnails and a better folder chooser would then provide the
largest visible gains. A full rewrite using native widgets is not necessary
to address those gaps.

## What already works

| Area | Current implementation and evidence |
| --- | --- |
| Actual desktop host | GTK application/windows, native file launching, GIO/GVfs access and a WebKit interface; `desktop/winspace.py` and `desktop/native_opening.py`. This is not a browser-only filesystem mock. |
| External and internal file dragging | Native URI/file transport, selected files/folders, copy proposals into folder/background targets, pinning and copy-only source semantics. See [the file interaction audit](FILE-INTERACTION-GAPS.md). Actual destination-app and session compatibility still matters. |
| Desktop clipboard | Copy/cut/paste and external GNOME/KDE formats, including the recent external-cut fixes; `desktop/file_clipboard.py`. |
| Navigation shortcuts | Alt+Left/Right/Up, Ctrl+L/Alt+D, Ctrl+F, F5, Ctrl+T/W/Tab/N, F2 and file commands; `desktop/ui/app.js`, `onKey`. Filename-prefix selection is implemented separately in `desktop/ui/type-select.js`. |
| Mouse navigation | Back/Forward buttons 3/4 are handled with dialog/auth guards (`desktop/ui/app.js:887`); middle-click opens background folders/tabs with a mouseup fallback for older WebKit. Do not describe these as absent. |
| Window and tab behavior | Real native windows, separate tab drag transport, acknowledged transfers, tab tear-out, Escape cancellation and explicit window menus. |
| Appearance | System light/dark changes reach the host and interface (`desktop/winspace.py:290`, `desktop/ui/app.js:614`). Text size, resizable columns/sidebar, reduced motion and some increased-contrast styling exist (`desktop/ui/style.css:15`, `:36`). |
| File changes | Opened directories get GIO monitors, a 350 ms debounce and refreshes; `desktop/winspace.py:1161`. The optional metadata index separately uses local inotify and incremental network checks (`desktop/index_service.py:96`, `:245`). |
| Transfer safety | Worker-based operations, progress, Cancel, operation results, non-overwriting copy staging and explicit permanent-delete confirmation where Trash is unavailable; `desktop/operations.py`, `desktop/ui/app.js:434`. |
| Basic accessibility | Semantic buttons and labelled inputs, menu/dialog roles, visible keyboard outlines, selection attributes, an authentication focus barrier and status announcements exist. This is a foundation, not an accessibility conformance claim. |

## Priorities for a launch candidate

These address correctness and trust before adding large features. The current
release label and validation limits should remain accurate while work proceeds.

| Priority | Gap and current evidence | Proposed result | Acceptance evidence |
| --- | --- | --- | --- |
| P0 | Keyboard navigation conflates the range anchor with the current item. In `desktop/ui/app.js:366` (`selectEntry`), Shift retains the anchor; `onKey` at `:923` calculates the next item from that same anchor. Repeated Shift+Down can therefore stop extending after one step. Grid Up/Down also use ±1; Left/Right and paging are absent. | Track current keyboard item independently of the range anchor. Support spatial grid movement, repeated range extension/contraction, empty selection, boundaries and resize/text-size changes. Keep input/menu/dialog keys isolated from file commands. | Exercise both views with enough rows to virtualize, Ctrl/Shift combinations, first/last items, type-to-select, tab changes and 80–200% text. Include actual WebKit keyboard behavior; Chromium tests alone establish only the HTML contract. |
| P1 | File-list accessibility does not expose a coherent current item. `renderRows` creates `role=row` nodes with `tabIndex=-1`, but `selectEntry` focuses the outer `main`. There is no active-descendant link; details cells lack gridcell roles, and icon tiles are each labelled as a row while `aria-rowcount` counts visual rows (`desktop/ui/app.js:351`, `desktop/ui/index.html:60–65`). | Choose one coherent file-list pattern and expose focus, selection and virtualized positions consistently. Preserve one convenient tab stop and distinct focused/selected states. Avoid adding ARIA attributes that contradict the actual row structure. | Inspect the accessibility tree and operate the installed app with Orca. Check names, position/count, multi-selection, sorting and offscreen navigation, not just DOM attributes. |
| P1 | Inactive tabs can become stale. The `changed` event only reloads the active matching tab (`desktop/ui/app.js:167`); `switchTab` at `:260` reloads only if `loaded` is false. An externally changed inactive tab is not invalidated. | Mark matching inactive tabs for refresh on activation. Preserve selection/scroll by URI where possible. Do not authenticate or enumerate every background network tab merely because a monitor event arrives. | Open two folders/tabs; externally add, rename and remove disposable files in the inactive one; switch back and confirm fresh contents. Repeat after a failed monitor and after reconnecting a share. |
| P1 | Regular dialogs do not match the authentication dialog's focus isolation. `showModal`/`closeModal` at `desktop/ui/app.js:401` use a hand-written focus loop, omit textarea/link targets from that loop and always return focus to `main`. Authentication explicitly uses `inert`, regular modal dialogs do not. | Restore focus to the invoking control when it still exists; consistently prevent interaction with covered content. Keep tab-scoped Properties behavior intentional. Include every enabled visible focusable control in keyboard containment. | Keyboard-only open/close/cancel through Properties, Rename, Extract, Open with and Settings. Verify Tab/Shift+Tab, Escape, error focus, nested auth and return to the triggering button. |
| P1 | Transfer progress is visual-only and can look like overall progress even though copy progress is per file. `desktop/ui/index.html:72` has a plain width bar; `updateTransfer` at `desktop/ui/app.js:453` only changes text/width. `desktop/operations.py:195` reports the current file's bytes and resets for later files. | Give the bar an accessible name/state and describe whether it is preparing, current-file progress or batch completion. Do not invent ETA, total bytes or monotonic whole-job progress without measuring them. Keep Cancel responsive and show partial completion honestly. | Inspect the accessibility tree during preparation, copying and cancellation. Test multiple unequal files and a directory; verify a 100% current-file bar is not presented as completion of the entire batch. |
| P1 | Remaining native integration coverage is narrower than the full user workflow. Protocol tests and simulated HTML do not prove the installed T3 Code build, target Wayland session or live SMB behavior. | Run a small supported-platform matrix before claiming stable native interoperability. A public release-candidate website can communicate the current limits without claiming complete explorer parity. | Install/start/restart/upgrade; real file drop into intended editor; clipboard with a standard file manager; scale/font changes; disconnect/reconnect; permission/disk-full/cancel checks; and keyring/auth flows on disposable data. Record exact environments and failures. |

The WAI grid guidance calls for a managed focus model and directional
navigation; the current combination of visual selection and outer-container
focus deserves a real assistive-technology check. The final semantic pattern
should reflect whether the interface is a file list or a genuine row/cell
grid. [WAI grid pattern](https://www.w3.org/WAI/ARIA/apg/patterns/grid/)

## Features to add after those corrections

| Priority | Feature | Current boundary and recommended scope |
| --- | --- | --- |
| P2 | Thumbnails and useful previews | `desktop/ui/app.js:381` (`renderDetails`) draws a file-type icon, not image/document content. Start with bounded local image thumbnails and selected-file image/text previews, loaded off the GTK thread. Cancel stale work when selection changes; cap memory/file sizes and reuse validated cached thumbnails where practical. Make remote preview downloads explicit or configurable. PDFs/video can follow through isolated providers. |
| P2 | Native destination chooser | Extract and folder-location flows currently use typed path fields, e.g. `desktop/ui/app.js`, `extractDialog`. Add a Browse button backed by an appropriate GTK/portal chooser while retaining editable local/SMB paths. Choosing a directory must not silently relocate files, mount a share or change defaults. |
| P2 | System font and desktop styling preferences | `desktop/ui/style.css:2` hard-codes a Segoe/Noto/Arial stack; the host observes color scheme but not system font preferences. Read the desktop font/size and relevant accessibility preferences into the native UI, with an explicit app override. Keep the project's Windows-inspired visual direction; native behavior does not require replacing it. GTK exposes a default UI font setting. [GTK font setting](https://docs.gtk.org/gtk3/property.Settings.gtk-font-name.html) |
| P2 | Faster selection and tab continuity | Ctrl/Shift click and Select all exist. Rubber-band selection, checkbox selection, multi-path Copy as path and preserving ordinary selection when switching tabs do not. `switchTab` explicitly clears the shared selection; `copyPath` at `desktop/ui/app.js:423` handles one selected item or the directory. Introduce per-tab focus/selection state first, then optional selection conveniences. |
| P2 | Trash access and restore | GIO Trash operations exist (`desktop/gio_backend.py:116`), but there is no Trash browsing/restore route in the interface; `core.normalise_location` accepts only file/SMB locations. Initially offer an explicit route to the system Trash and clear recovery guidance. A full in-app restore flow needs conflict/original-location handling. Preserve the separate explicit permanent-delete flow for locations without Trash. |
| P2 | Menu polish and native app icons | Context menus and Open with already exist, including both menu styles, keyboard access and real application lookup. Improve focus return, menu/typeahead consistency, application icons and disabled-action explanations. Do not replace functioning menus merely because they are rendered by WebKit. |
| P2 | Monitoring health and stale-state feedback | The browsing window retains at most eight GIO directory monitors and silently ignores unsupported monitor creation (`desktop/winspace.py:1161`). Search indexing has different limits and coverage. Surface refresh/freshness when monitoring is unavailable, mark tabs dirty on events and use bounded fallback refresh where appropriate. GIO directory-monitor availability depends on backend support. [GIO monitor API](https://docs.gtk.org/gio/method.File.monitor_directory.html) |
| P3 | Persistent operation history and queue | The interface exposes one active operation per window and a completion/error dialog. A later operation center could retain failures and completed paths, coordinate queued work and give clear per-job cancellation. Start with honest history before adding pause/resume promises. |
| P3 | Undo, cross-filesystem moves and per-item conflict review | Explicit Replace/Skip and same-name folder merging are implemented with staged copies and native-only moves. There is still no undo journal, durable restart recovery or per-item conflict picker. Do not implement a cross-filesystem move as unverified copy-then-delete. |

## Two bounded improvements suitable for immediate assignment

1. **Keyboard selection/navigation repair.** Implement independent current-item
   and range-anchor state, correct icon-grid directions and add focused
   regression cases for repeated Shift navigation. Scope keyboard actions to
   the file-list context so toolbar/menu controls retain their expected behavior.
   This improves a daily workflow without introducing filesystem writes.
2. **Accessible, accurately labelled transfer feedback.** Add semantic progress
   state tied to the existing events and distinguish current-file/preparing
   status. Reuse existing cancellation/result handling. Avoid a new transfer
   engine, ETA estimator, operation queue or undo implementation in this change.

The inactive-tab freshness issue is another small follow-up worth scheduling
immediately after those, or instead of the progress change if stale listings
are a frequent user complaint.

## Release preparation and evidence

Use [the release checklist](RELEASE-CHECKLIST.md) and the current executed test
report for installation/support claims. Keep AGPL corresponding source, notices,
the website source link, generated artifacts and package bytes synchronized.
Keep maintainer identity, source URL, supported distributions and update/release
instructions concrete before presenting a stable release. Do not rename
persisted `winspace` identifiers as a cosmetic cleanup.

This roadmap was prepared from source inspection and the linked primary
documentation. No new desktop, accessibility, filesystem or browser tests were
run for this roadmap. Refer to the separate test reports for checks actually
executed by the implementing agents. No screenshots or customer data were
captured; future public examples must follow [the privacy rules](PRIVACY.md).

Project-authored documentation: AGPL-3.0-only.
