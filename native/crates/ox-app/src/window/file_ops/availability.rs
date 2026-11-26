// SPDX-License-Identifier: AGPL-3.0-only
//! When each file command is enabled (CMD-002).
//!
//! Ports `updateToolbar` in `v2.0.0:desktop/ui/app.js`. The rules read a few
//! facts about the selection, the folder and the running operation
//! ([`CommandFacts`]) and decide each [`FileCommand`]; the window gathers
//! the facts and enables the window actions, so the command bar, the
//! menus and the keyboard shortcuts all follow the same rules. The Python
//! rules, word for word:
//!
//! - Copy: something is selected, every selected item can be operated on,
//!   and no operation runs.
//! - Cut and Delete: as Copy, and nothing selected is read-only.
//! - Rename: as Cut, with exactly one item selected. The native app also
//!   takes several: F2 gives them one numbered name, as Windows Explorer
//!   does (OPS-050), and Rename items… (the same command) opens the batch
//!   rename (OPS-014).
//! - Paste: a file clipboard, or a picture to write as one new file
//!   (CLIP-015), no search, no operation, every selected item operable, and
//!   a writable folder.
//! - New: no search, no operation, and a writable folder.
//!
//! The commands the Python app did not have follow the nearest rule:
//! Shift+Delete and Duplicate as Delete, Undo and Redo while no operation
//! runs, and the Recycle Bin's Restore, Restore all items and Empty
//! inside it, the last two while it holds items. In the Recycle
//! Bin, Cut, Copy, Rename and Duplicate are off: its items can only be
//! restored or deleted.

use gtk::gio::prelude::FileExt;
use gtk::subclass::prelude::*;
use ox_core::location::{is_archive_location, is_cloud_location, same_location, LocationContext, TRASH_URI};
use ox_core::ops::JournalDirection;

use crate::folder_view::item::FileItem;
use crate::window::command_bar::RestoreTarget;
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

use super::ClipboardKind;

/// A command whose enabled state these rules decide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileCommand {
    /// Every item of the New menu, and the menu itself.
    New,
    /// Cut (Ctrl+X).
    Cut,
    /// Copy (Ctrl+C).
    Copy,
    /// Paste (Ctrl+V).
    Paste,
    /// Paste into the one selected folder (CMD-019).
    PasteInto,
    /// Rename (F2).
    Rename,
    /// Delete: Move to Trash, or Delete permanently without a Trash.
    Delete,
    /// Shift+Delete.
    DeletePermanently,
    /// Duplicate.
    Duplicate,
    /// Undo (Ctrl+Z).
    Undo,
    /// Redo (Ctrl+Shift+Z).
    Redo,
    /// Restore, in the Recycle Bin.
    Restore,
    /// Restore all items, in the Recycle Bin.
    RestoreAll,
    /// Empty Recycle Bin.
    EmptyRecycleBin,
    /// Cancel on the transfer panel.
    CancelOperation,
}

impl FileCommand {
    /// Every command, for enabling them all at once.
    pub(crate) const ALL: [FileCommand; 15] = [
        FileCommand::New,
        FileCommand::Cut,
        FileCommand::Copy,
        FileCommand::Paste,
        FileCommand::PasteInto,
        FileCommand::Rename,
        FileCommand::Delete,
        FileCommand::DeletePermanently,
        FileCommand::Duplicate,
        FileCommand::Undo,
        FileCommand::Redo,
        FileCommand::Restore,
        FileCommand::RestoreAll,
        FileCommand::EmptyRecycleBin,
        FileCommand::CancelOperation,
    ];

    /// The window actions the command enables and disables.
    pub(crate) const fn actions(self) -> &'static [WindowAction] {
        match self {
            FileCommand::New => &[
                WindowAction::NewFolder,
                WindowAction::NewLink,
                WindowAction::NewBitmapImage,
                WindowAction::NewRichTextDocument,
                WindowAction::NewTextDocument,
                WindowAction::NewCompressedFolder,
                WindowAction::NewOpenDocument,
                WindowAction::NewFromUserTemplate,
                WindowAction::ShowNewMenu,
            ],
            FileCommand::Cut => &[WindowAction::Cut],
            FileCommand::Copy => &[WindowAction::Copy],
            FileCommand::Paste => &[WindowAction::Paste],
            FileCommand::PasteInto => &[WindowAction::PasteInto],
            FileCommand::Rename => &[WindowAction::Rename, WindowAction::RenameItems],
            FileCommand::Delete => &[WindowAction::Trash],
            FileCommand::DeletePermanently => &[WindowAction::DeletePermanently],
            FileCommand::Duplicate => &[WindowAction::Duplicate],
            FileCommand::Undo => &[WindowAction::Undo],
            FileCommand::Redo => &[WindowAction::Redo],
            FileCommand::Restore => &[WindowAction::Restore],
            FileCommand::RestoreAll => &[WindowAction::RestoreAllItems],
            FileCommand::EmptyRecycleBin => &[WindowAction::EmptyRecycleBin],
            FileCommand::CancelOperation => &[WindowAction::CancelOperation],
        }
    }
}

/// What the selection is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SelectionFacts {
    /// How many items are selected.
    pub(crate) count: usize,
    /// Some selected item cannot be operated on: a share or device root,
    /// or a virtual entry (`canOperate`).
    pub(crate) has_inoperable: bool,
    /// Some selected item is in a read-only previous version
    /// (`readonlyLocation`).
    pub(crate) has_read_only: bool,
    /// Some selected item is a row of Recent files, which may be copied,
    /// not cut, duplicated, renamed or deleted.
    pub(crate) has_recent: bool,
}

/// What the folder the tab shows is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is one independent fact that updateToolbar reads, not a state"
)]
pub(crate) struct FolderFacts {
    /// New and Paste may create items in it (`writableLocation`).
    pub(crate) is_writable: bool,
    /// The search box filters it (`state.query`).
    pub(crate) is_searching: bool,
    /// It is the Recycle Bin itself.
    pub(crate) is_recycle_bin: bool,
    /// It shows at least one item, or it is the Recycle Bin and holds
    /// items a search hides.
    pub(crate) has_items: bool,
    /// It is inside a ZIP opened like a folder, read-only (ARC-026).
    pub(crate) is_zip: bool,
    /// Its drive's file system may not be written, as a write-protected
    /// USB stick's.
    pub(crate) is_on_read_only_drive: bool,
    /// It is on Google Drive or `OneDrive`, read-only for now (DEV-013).
    pub(crate) is_cloud: bool,
}

/// Everything the rules read.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each flag is one independent fact that updateToolbar reads, not a state"
)]
pub(crate) struct CommandFacts {
    /// The selection.
    pub(crate) selection: SelectionFacts,
    /// The folder.
    pub(crate) folder: FolderFacts,
    /// An operation runs or a paste is being planned (`state.operation`).
    pub(crate) is_busy: bool,
    /// What the desktop's clipboard holds: files, a picture or nothing
    /// (`state.clipboard`, CLIP-009, CLIP-015).
    pub(crate) clipboard: ClipboardKind,
    /// Undo has something to reverse.
    pub(crate) can_undo: bool,
    /// Redo has something to take back.
    pub(crate) can_redo: bool,
    /// A drive's tile is selected on This PC, which Copy copies: a paste
    /// of it makes a shortcut to the drive, as in Windows Explorer.
    pub(crate) drive_selected: bool,
    /// Whether a row of Home's Recent or Favorites is selected, whose file
    /// Copy acts on (HOME-036).
    pub(crate) home_file_selected: bool,
    /// Whether that row is a Favorites row, whose file Cut, Rename and
    /// Delete also act on; Windows 11 offers none of them on a Recent row
    /// (checked on Windows 11, 2026-11-22).
    pub(crate) home_file_changeable: bool,
}

impl CommandFacts {
    /// Whether `command` is enabled.
    pub(crate) fn allows(&self, command: FileCommand) -> bool {
        let selection = self.selection;
        let folder = self.folder;
        // `busy` in updateToolbar: an operation runs, or an item cannot be
        // operated on.
        let busy = self.is_busy || selection.has_inoperable;
        let can_copy = selection.count > 0 && !busy && !folder.is_recycle_bin;
        let can_change = selection.count > 0 && !busy && !selection.has_read_only;
        match command {
            FileCommand::New => {
                !folder.is_searching && !self.is_busy && folder.is_writable && !folder.is_on_read_only_drive
            }
            FileCommand::Copy => {
                can_copy || ((self.drive_selected || self.home_file_selected) && !self.is_busy)
            }
            FileCommand::Cut | FileCommand::Rename => {
                (can_copy && !selection.has_read_only && !selection.has_recent)
                    || (self.home_file_changeable && !self.is_busy)
            }
            FileCommand::Duplicate => can_copy && !selection.has_read_only && !selection.has_recent,
            FileCommand::Delete | FileCommand::DeletePermanently => {
                (can_change && !selection.has_recent) || (self.home_file_changeable && !self.is_busy)
            }
            FileCommand::Paste => {
                self.clipboard.is_pasteable() && !folder.is_searching && !busy && folder.is_writable
            }
            // The selected folder is the destination, so a search does not
            // matter; whether it takes items is checked when pasting.
            FileCommand::PasteInto => {
                let is_one_item = selection.count == 1 && !selection.has_read_only;
                self.clipboard.is_pasteable() && !busy && !folder.is_recycle_bin && is_one_item
            }
            FileCommand::Undo => self.can_undo && !self.is_busy,
            FileCommand::Redo => self.can_redo && !self.is_busy,
            FileCommand::Restore => folder.is_recycle_bin && can_change,
            FileCommand::RestoreAll | FileCommand::EmptyRecycleBin => {
                folder.is_recycle_bin && folder.has_items && !self.is_busy
            }
            FileCommand::CancelOperation => self.is_busy,
        }
    }

    /// Why `command` is disabled, in the words a menu item's accessible
    /// description or a command bar button's tooltip gives; `None` while
    /// it is enabled.
    pub(crate) fn refusal(&self, command: FileCommand) -> Option<&'static str> {
        if self.allows(command) {
            return None;
        }
        let selection = self.selection;
        let folder = self.folder;
        let reason = match command {
            FileCommand::CancelOperation => ox_core::i18n::gettext_static("No file operation is running."),
            FileCommand::Undo if !self.can_undo => ox_core::i18n::gettext_static("Nothing to undo."),
            FileCommand::Redo if !self.can_redo => ox_core::i18n::gettext_static("Nothing to redo."),
            // A picture is pasted as a file (CLIP-015); text, as in
            // Windows Explorer, is nothing to paste.
            FileCommand::Paste | FileCommand::PasteInto if self.clipboard == ClipboardKind::TooLarge => {
                ox_core::i18n::gettext_static(ox_core::clipboard::TOO_LARGE_TO_PASTE)
            }
            FileCommand::Paste | FileCommand::PasteInto if !self.clipboard.is_pasteable() => {
                ox_core::i18n::gettext_static("Nothing to paste here.")
            }
            FileCommand::Restore | FileCommand::RestoreAll if !folder.is_recycle_bin => {
                ox_core::i18n::gettext_static("Only items in the Recycle Bin can be restored.")
            }
            FileCommand::EmptyRecycleBin if !folder.is_recycle_bin => {
                ox_core::i18n::gettext_static("This is not the Recycle Bin.")
            }
            FileCommand::RestoreAll | FileCommand::EmptyRecycleBin if !folder.has_items => {
                ox_core::i18n::gettext_static("The Recycle Bin is empty.")
            }
            _ if self.is_busy => {
                ox_core::i18n::gettext_static("Wait for the running file operation to finish.")
            }
            FileCommand::New | FileCommand::Paste if folder.is_searching => {
                ox_core::i18n::gettext_static("Clear the search to add items to this folder.")
            }
            FileCommand::New | FileCommand::Paste if folder.is_zip => {
                ox_core::i18n::gettext_static("An archive is read-only. Extract it to add items.")
            }
            FileCommand::New | FileCommand::Paste if folder.is_cloud => ox_core::i18n::gettext_static(
                "Google Drive and OneDrive are read-only for now. Copy items to this PC to change them.",
            ),
            FileCommand::New => ox_core::i18n::gettext_static("This folder is read-only."),
            FileCommand::Paste if !selection.has_inoperable => {
                ox_core::i18n::gettext_static("This folder is read-only.")
            }
            FileCommand::PasteInto if selection.count != 1 => {
                ox_core::i18n::gettext_static("Select one folder to paste into.")
            }
            _ if selection.count == 0 => ox_core::i18n::gettext_static("Select an item first."),
            _ if selection.has_inoperable => {
                ox_core::i18n::gettext_static("Drives, shares and virtual items cannot be changed here.")
            }
            FileCommand::Cut
            | FileCommand::Duplicate
            | FileCommand::Rename
            | FileCommand::Delete
            | FileCommand::DeletePermanently
                if selection.has_recent =>
            {
                ox_core::i18n::gettext_static("Items in Recent files can be opened and copied, not changed.")
            }
            FileCommand::Copy | FileCommand::Cut | FileCommand::Rename | FileCommand::Duplicate
                if folder.is_recycle_bin =>
            {
                ox_core::i18n::gettext_static("Items in the Recycle Bin can only be restored or deleted.")
            }
            _ if selection.has_read_only && folder.is_zip => ox_core::i18n::gettext_static(
                "Items in an archive are read-only. Copy them out or use Extract all.",
            ),
            _ if selection.has_read_only && folder.is_cloud => ox_core::i18n::gettext_static(
                "Google Drive and OneDrive are read-only for now. Copy items to this PC to change them.",
            ),
            _ if selection.has_read_only => ox_core::i18n::gettext_static("A previous version is read-only."),
            _ => ox_core::i18n::gettext_static("Rename one item at a time."),
        };
        Some(reason)
    }
}

/// Whether the jobs that run keep `command` waiting.
///
/// A copy, a cut and a paste of a file list queue behind them, up to
/// [`MAX_JOBS`](super::jobs::MAX_JOBS). A picture is written as the
/// window's own quick write, which runs only while no job does, so such a
/// paste waits for them instead: enabled, it would do nothing (CLIP-015).
fn waits_for_running_jobs(command: FileCommand, clipboard: ClipboardKind, jobs: usize) -> bool {
    match command {
        FileCommand::Copy | FileCommand::Cut => false,
        FileCommand::Paste | FileCommand::PasteInto if clipboard != ClipboardKind::Files => true,
        FileCommand::Paste
        | FileCommand::PasteInto
        | FileCommand::Delete
        | FileCommand::DeletePermanently => jobs >= super::jobs::MAX_JOBS,
        _ => true,
    }
}

/// The facts about `items`, the selected items (`canOperate` and
/// `readonlyLocation` in app.js).
pub(crate) fn selection_facts(items: &[FileItem], locations: &LocationContext) -> SelectionFacts {
    // A row of Recent files that names its file is copied as that file,
    // whether `GVfs` types it as a file or as a shortcut; its Cut,
    // Duplicate, Rename and Delete are refused as `has_recent`.
    // What each item's address says is worked out once per item
    // (`AddressFacts`), so updating the commands for 100,000 selected items
    // does not split and decode 100,000 addresses each time.
    let has_inoperable = items.iter().any(|item| {
        let entry = item.entry();
        let stands_for_a_file = entry.meta.recent_target.is_some();
        !stands_for_a_file && (!entry.can_operate || entry.is_virtual || item.address_facts().smb_share_root)
    });
    let has_snapshot_roots = !locations.snapshot_roots.is_empty();
    let has_read_only = items.iter().any(|item| {
        item.address_facts().read_only_by_path
            || (has_snapshot_roots && locations.is_snapshot_location(&item.entry().uri))
    });
    let has_recent = items.iter().any(|item| item.address_facts().recent_row);
    SelectionFacts {
        count: items.len(),
        has_inoperable,
        has_read_only,
        has_recent,
    }
}

impl BrowserWindow {
    /// True while the folder shown could not be read ("This location is
    /// unavailable"): its listing failed, or it is listed again from that
    /// page and has not been read yet. It takes no new items then, as it
    /// takes no drop.
    pub(crate) fn shown_folder_is_unreadable(&self) -> bool {
        let failed = {
            let session = self.imp().session.borrow();
            session.active().is_some_and(|tab| tab.error.is_some())
        };
        failed || self.folder_pane().shows_unavailable_page()
    }

    /// What Restore in the command bar restores while the tab shows the
    /// Recycle Bin: the selected items, else all of them; `None` elsewhere.
    fn recycle_bin_restore_target(&self) -> Option<RestoreTarget> {
        let facts = self.command_facts();
        facts
            .folder
            .is_recycle_bin
            .then_some(if facts.selection.count > 0 {
                RestoreTarget::SelectedItems
            } else {
                RestoreTarget::AllItems
            })
    }

    /// The facts the rules read now.
    pub(crate) fn command_facts(&self) -> CommandFacts {
        let model = self.folder_pane().model();
        let locations = self.imp().locations.borrow();
        let folder_uri = self.current_uri().unwrap_or_default();
        let listing_failed = self.shown_folder_is_unreadable();
        let is_recycle_bin = same_location(&folder_uri, TRASH_URI);
        let folder = FolderFacts {
            is_writable: locations.is_writable_location(&folder_uri) && !listing_failed,
            is_searching: self.is_searching(),
            is_recycle_bin,
            // A search in the Recycle Bin hides items it still holds,
            // which Restore all items and Empty Recycle Bin act on.
            has_items: model.n_items() > 0
                || (is_recycle_bin && self.is_searching() && self.imp().trash_items.get() > 0),
            is_zip: is_archive_location(&folder_uri),
            is_on_read_only_drive: {
                let known = self.imp().read_only_drive.borrow();
                known.1 && known.0 == folder_uri
            },
            is_cloud: is_cloud_location(&folder_uri),
        };
        let operations = self.imp().file_operations.borrow();
        let context = self.context();
        CommandFacts {
            selection: selection_facts(&model.selected_items(), &locations),
            folder,
            is_busy: operations.is_busy(),
            clipboard: operations.clipboard,
            can_undo: context.journal_label(JournalDirection::Undo).is_some(),
            can_redo: context.journal_label(JournalDirection::Redo).is_some(),
            drive_selected: self
                .selected_drive_tiles()
                .iter()
                .any(|uri| !ox_core::location::is_device_location(uri)),
            home_file_selected: self.selected_home_file().is_some(),
            home_file_changeable: self.selected_home_file().is_some() && !self.selected_home_file_is_recent(),
        }
    }

    /// Whether `command` is enabled now.
    pub(crate) fn allows(&self, command: FileCommand) -> bool {
        self.facts_for_command(command).allows(command)
    }

    pub(crate) fn facts_for_command(&self, command: FileCommand) -> CommandFacts {
        let mut facts = self.command_facts();
        let jobs = self.imp().file_operations.borrow().jobs.len();
        if jobs > 0 {
            facts.is_busy |= waits_for_running_jobs(command, facts.clipboard, jobs);
        }
        facts
    }

    /// Asks, once per folder shown and off the main loop, whether the
    /// folder's file system is read-only, as a write-protected USB stick
    /// is; the answer greys New and updates the commands.
    fn learn_whether_the_drive_is_read_only(&self) {
        let Some(uri) = self.current_uri() else {
            return;
        };
        if self.imp().read_only_drive.borrow().0 == uri {
            return;
        }
        self.imp().read_only_drive.replace((uri.clone(), false));
        self.ask_whether_read_only(uri);
    }

    /// Asks again whether the drive of the folder shown is read-only, once
    /// the folder was listed anew (F5, a share reconnected, a drive
    /// mounted again): the drive at the same address may have changed,
    /// as a USB stick plugged back with its write-protect switch moved,
    /// and a first question that failed is asked again. The answer known
    /// holds until the new one comes, so New does not flicker.
    pub(in crate::window) fn recheck_read_only_drive(&self) {
        let Some(uri) = self.current_uri() else {
            return;
        };
        if self.imp().read_only_drive.borrow().0 == uri {
            self.ask_whether_read_only(uri);
        }
    }

    /// Asks off the main loop whether the file system of the folder at
    /// `uri` is read-only, and keeps the answer while that folder is shown,
    /// updating the commands when it changed. A question that fails
    /// changes nothing.
    fn ask_whether_read_only(&self, uri: String) {
        let folder = gtk::gio::File::for_uri(&uri);
        gtk::glib::spawn_future_local(gtk::glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                let Ok(info) = folder
                    .query_filesystem_info_future(
                        gtk::gio::FILE_ATTRIBUTE_FILESYSTEM_READONLY,
                        gtk::glib::Priority::DEFAULT,
                    )
                    .await
                else {
                    return;
                };
                let read_only = info.boolean(gtk::gio::FILE_ATTRIBUTE_FILESYSTEM_READONLY);
                let known = window.imp().read_only_drive.borrow().clone();
                if known.0 == uri && known.1 != read_only {
                    window.imp().read_only_drive.replace((uri, read_only));
                    window.update_file_commands();
                }
            }
        ));
    }

    /// Enables and disables every file command, and labels Delete for the
    /// selection's folder (`updateToolbar`).
    pub(crate) fn update_file_commands(&self) {
        self.learn_whether_the_drive_is_read_only();
        for command in FileCommand::ALL {
            let enabled = self.allows(command);
            for &action in command.actions() {
                self.set_action_enabled(action, enabled);
            }
        }
        self.command_bar().set_new_enabled(self.allows(FileCommand::New));
        self.command_bar().show_delete_label(self.delete_label());
        self.command_bar()
            .show_recycle_bin_commands(self.recycle_bin_restore_target());
        self.show_gallery_commands();
        // "…" lists Undo and Redo only while each can run.
        self.update_more_menu();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Facts for a writable folder with `count` plain items selected and
    /// nothing running.
    fn selected(count: usize) -> CommandFacts {
        CommandFacts {
            selection: SelectionFacts {
                count,
                ..SelectionFacts::default()
            },
            folder: FolderFacts {
                is_writable: true,
                has_items: true,
                ..FolderFacts::default()
            },
            clipboard: ClipboardKind::Files,
            ..CommandFacts::default()
        }
    }

    /// The commands `facts` enables, in [`FileCommand::ALL`] order.
    fn enabled(facts: &CommandFacts) -> Vec<FileCommand> {
        FileCommand::ALL
            .into_iter()
            .filter(|command| facts.allows(*command))
            .collect()
    }

    fn address_reads() -> u32 {
        crate::folder_view::item::ADDRESS_READS.with(std::cell::Cell::get)
    }

    /// The commands are updated several times after each change of the
    /// selection; each item's address is read only the first time.
    ///
    /// parity: PERF-004
    #[gtk::test]
    fn each_selected_address_is_read_once() {
        let items: Vec<FileItem> = (0..500)
            .map(|number| FileItem::new(crate::test_support::file_entry(&format!("photo {number}.jpg"))))
            .collect();
        let locations = LocationContext::default();
        let before = address_reads();
        for _ in 0..4 {
            let facts = selection_facts(&items, &locations);
            assert_eq!(facts.count, 500);
            assert!(!facts.has_inoperable && !facts.has_read_only && !facts.has_recent);
        }
        assert_eq!(address_reads() - before, 500);
    }

    /// What an address says stays right with the reading kept: a snapshot
    /// by its path, a ZIP, a share, a row of Recent files, and a folder the
    /// settings name a snapshot root, which is checked each time.
    ///
    /// parity: PERF-004
    #[gtk::test]
    fn kept_address_facts_still_mark_read_only_and_inoperable_items() {
        let item = |uri: &str| {
            let mut entry = crate::test_support::file_entry("item");
            entry.uri = uri.to_owned();
            FileItem::new(entry)
        };
        let plain = LocationContext::default();
        let facts = |uri: &str, locations: &LocationContext| selection_facts(&[item(uri)], locations);
        assert!(facts("file:///srv/data/.snapshot/daily/a.txt", &plain).has_read_only);
        let inside_zip = ox_core::location::ArchiveLocation::root("file:///srv/a.zip")
            .member("inside.txt")
            .uri();
        assert!(facts(&inside_zip, &plain).has_read_only);
        assert!(facts("smb://studio-nas/projects", &plain).has_inoperable);
        assert!(facts("recent:///abc", &plain).has_recent);
        assert!(!facts("file:///srv/data/a.txt", &plain).has_read_only);
        let rooted = LocationContext {
            snapshot_roots: vec!["file:///srv/backup".to_owned()],
            ..LocationContext::default()
        };
        assert!(facts("file:///srv/backup/a.txt", &rooted).has_read_only);
    }

    /// parity: CMD-002
    #[test]
    fn one_selected_item_enables_every_edit_command() {
        let facts = selected(1);

        assert_eq!(
            enabled(&facts),
            [
                FileCommand::New,
                FileCommand::Cut,
                FileCommand::Copy,
                FileCommand::Paste,
                FileCommand::PasteInto,
                FileCommand::Rename,
                FileCommand::Delete,
                FileCommand::DeletePermanently,
                FileCommand::Duplicate,
            ]
        );
    }

    /// A drive selected on This PC can be copied, never cut, renamed or
    /// deleted, as in Windows Explorer; Copy waits for nothing but a
    /// running operation.
    #[test]
    fn a_selected_drive_enables_copy_alone() {
        let drive = CommandFacts {
            drive_selected: true,
            home_file_selected: false,
            home_file_changeable: false,
            ..CommandFacts::default()
        };
        let mut busy = drive;
        busy.is_busy = true;

        assert_eq!(enabled(&drive), [FileCommand::Copy]);
        assert_eq!(drive.refusal(FileCommand::Copy), None);
        assert!(drive.refusal(FileCommand::Cut).is_some());
        assert!(!busy.allows(FileCommand::Copy));
    }

    /// Every disabled command says why, and an enabled one says nothing.
    ///
    /// parity: CMD-031
    #[test]
    fn a_disabled_command_says_why() {
        let none = selected(0);
        let mut busy = selected(1);
        busy.is_busy = true;
        let mut searching = selected(0);
        searching.folder.is_searching = true;

        assert_eq!(none.refusal(FileCommand::Copy), Some("Select an item first."));
        // Several items are renamed together (OPS-014, OPS-050).
        assert_eq!(selected(2).refusal(FileCommand::Rename), None);
        assert_eq!(selected(1).refusal(FileCommand::Rename), None);
        assert_eq!(
            busy.refusal(FileCommand::Delete),
            Some("Wait for the running file operation to finish.")
        );
        assert_eq!(
            searching.refusal(FileCommand::New),
            Some("Clear the search to add items to this folder.")
        );
        for facts in [none, selected(2), busy, searching] {
            for command in FileCommand::ALL {
                assert_eq!(
                    facts.refusal(command).is_some(),
                    !facts.allows(command),
                    "{command:?}"
                );
            }
        }
    }

    /// parity: CMD-002, OPS-014, OPS-050
    #[test]
    fn several_items_are_renamed_together_and_nothing_needs_none() {
        let two = selected(2);
        let none = selected(0);

        assert!(two.allows(FileCommand::Rename));
        assert!(two.allows(FileCommand::Copy));
        assert_eq!(enabled(&none), [FileCommand::New, FileCommand::Paste]);
    }

    /// parity: CMD-002
    #[test]
    fn a_read_only_item_can_be_copied_but_not_changed() {
        let mut facts = selected(1);
        facts.selection.has_read_only = true;

        assert!(facts.allows(FileCommand::Copy));
        for command in [
            FileCommand::Cut,
            FileCommand::Rename,
            FileCommand::Delete,
            FileCommand::DeletePermanently,
            FileCommand::Duplicate,
        ] {
            assert!(!facts.allows(command), "{command:?}");
        }
    }

    /// parity: CMD-002, OPS-024, OPS-035
    #[test]
    fn a_running_operation_or_a_share_root_disables_the_edit_commands() {
        let mut running = selected(1);
        running.is_busy = true;
        let mut share_root = selected(1);
        share_root.selection.has_inoperable = true;

        assert_eq!(enabled(&running), [FileCommand::CancelOperation]);
        assert_eq!(enabled(&share_root), [FileCommand::New]);
    }

    /// parity: CMD-002
    #[test]
    fn search_and_folders_that_are_not_writable_disable_new_and_paste() {
        let mut searching = selected(0);
        searching.folder.is_searching = true;
        let mut read_only_folder = selected(0);
        read_only_folder.folder.is_writable = false;

        assert!(enabled(&searching).is_empty());
        assert!(enabled(&read_only_folder).is_empty());
    }

    /// On a drive that may not be written, New is greyed, as Windows 11
    /// greys it on a write-protected USB stick (checked on a Windows 11
    /// VM); everything else follows its own rule, so Paste stays on and
    /// says the system's error.
    ///
    /// parity: DEV-015
    #[test]
    fn a_read_only_drive_greys_new() {
        let mut stick = selected(0);
        stick.folder.is_on_read_only_drive = true;

        assert_eq!(enabled(&stick), [FileCommand::Paste]);
        assert_eq!(stick.refusal(FileCommand::New), Some("This folder is read-only."));
    }

    /// A paste of a file list queues behind the jobs that run, as a copy does;
    /// one of a picture writes its file as the window's own quick write,
    /// which runs only while no job does, so it waits for them (CLIP-015).
    #[test]
    fn a_paste_of_data_waits_for_the_jobs_that_run() {
        let max = super::super::jobs::MAX_JOBS;
        assert!(!waits_for_running_jobs(
            FileCommand::Paste,
            ClipboardKind::Files,
            max - 1
        ));
        assert!(waits_for_running_jobs(
            FileCommand::Paste,
            ClipboardKind::Files,
            max
        ));
        assert!(
            waits_for_running_jobs(FileCommand::Paste, ClipboardKind::Image, 1),
            "a picture waits for the first job"
        );
        assert!(waits_for_running_jobs(
            FileCommand::PasteInto,
            ClipboardKind::Image,
            1
        ));
        assert!(!waits_for_running_jobs(
            FileCommand::Copy,
            ClipboardKind::Files,
            1
        ));
    }

    /// A picture is pasted as one new file, so it enables Paste as a file
    /// list does; a clipboard with neither has nothing to write.
    ///
    /// parity: CLIP-015, CLIP-016
    #[test]
    fn paste_needs_files_or_a_picture_on_the_clipboard() {
        let mut facts = selected(0);
        facts.clipboard = ClipboardKind::Empty;

        assert!(!facts.allows(FileCommand::Paste));
        assert!(!facts.allows(FileCommand::PasteInto));
        assert_eq!(facts.refusal(FileCommand::Paste), Some("Nothing to paste here."));
        for kind in [ClipboardKind::Files, ClipboardKind::Image] {
            let mut holds = selected(0);
            holds.clipboard = kind;
            assert!(holds.allows(FileCommand::Paste), "{kind:?}");
        }
    }

    /// A clipboard past the size the app accepts refuses Paste in its own
    /// words, so a Ctrl+V that does nothing says why.
    ///
    /// parity: CLIP-015
    #[test]
    fn a_clipboard_over_the_size_limit_says_why_paste_is_off() {
        let mut facts = selected(0);
        facts.clipboard = ClipboardKind::TooLarge;

        assert!(!facts.allows(FileCommand::Paste));
        assert_eq!(
            facts.refusal(FileCommand::Paste),
            Some(ox_core::clipboard::TOO_LARGE_TO_PASTE)
        );
    }

    /// parity: OPS-029, OPS-031
    #[test]
    fn undo_and_redo_follow_the_journal_while_nothing_runs() {
        let mut facts = selected(0);
        facts.can_undo = true;
        facts.can_redo = true;
        let mut running = facts;
        running.is_busy = true;

        assert!(facts.allows(FileCommand::Undo));
        assert!(facts.allows(FileCommand::Redo));
        assert!(!running.allows(FileCommand::Undo));
        assert!(!running.allows(FileCommand::Redo));
    }

    /// parity: OPS-041, OPS-042
    #[test]
    fn the_recycle_bin_restores_and_empties_but_does_not_copy() {
        let mut facts = selected(1);
        facts.folder.is_recycle_bin = true;
        facts.folder.is_writable = false;
        let mut empty = selected(0);
        empty.folder.is_recycle_bin = true;
        empty.folder.has_items = false;

        assert_eq!(
            enabled(&facts),
            [
                FileCommand::Delete,
                FileCommand::DeletePermanently,
                FileCommand::Restore,
                FileCommand::RestoreAll,
                FileCommand::EmptyRecycleBin,
            ]
        );
        assert!(!empty.allows(FileCommand::EmptyRecycleBin));
        assert!(!empty.allows(FileCommand::RestoreAll));
    }
}
