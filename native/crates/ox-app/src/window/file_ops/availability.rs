// SPDX-License-Identifier: AGPL-3.0-only
//! When each file command is enabled (CMD-002).
//!
//! Ports `updateToolbar` in `desktop/ui/app.js`. The rules read a few
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
//!   takes several, for the batch rename (OPS-014).
//! - Paste: a file clipboard, no search, no operation, every selected item
//!   operable, and a writable folder.
//! - New: no search, no operation, and a writable folder.
//!
//! The commands the Python app did not have follow the nearest rule:
//! Shift+Delete and Duplicate as Delete, Undo and Redo while no operation
//! runs, and the Recycle Bin's Restore and Empty inside it. In the Recycle
//! Bin, Cut, Copy, Rename and Duplicate are off: its items can only be
//! restored or deleted.

use gtk::subclass::prelude::*;
use ox_core::location::{is_smb_share_root, same_location, LocationContext, TRASH_URI};
use ox_core::ops::JournalDirection;

use crate::folder_view::item::FileItem;
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

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
    /// Empty Recycle Bin.
    EmptyRecycleBin,
    /// Cancel on the transfer panel.
    CancelOperation,
}

impl FileCommand {
    /// Every command, for enabling them all at once.
    pub(crate) const ALL: [FileCommand; 13] = [
        FileCommand::New,
        FileCommand::Cut,
        FileCommand::Copy,
        FileCommand::Paste,
        FileCommand::Rename,
        FileCommand::Delete,
        FileCommand::DeletePermanently,
        FileCommand::Duplicate,
        FileCommand::Undo,
        FileCommand::Redo,
        FileCommand::Restore,
        FileCommand::EmptyRecycleBin,
        FileCommand::CancelOperation,
    ];

    /// The window actions the command enables and disables.
    pub(crate) const fn actions(self) -> &'static [WindowAction] {
        match self {
            FileCommand::New => &[
                WindowAction::NewFolder,
                WindowAction::NewTextDocument,
                WindowAction::NewFile,
                WindowAction::NewMarkdownDocument,
                WindowAction::NewCsvFile,
                WindowAction::NewJsonFile,
                WindowAction::NewHtmlDocument,
                WindowAction::NewFromTemplate,
                WindowAction::NewLink,
                WindowAction::ShowNewMenu,
            ],
            FileCommand::Cut => &[WindowAction::Cut],
            FileCommand::Copy => &[WindowAction::Copy],
            FileCommand::Paste => &[WindowAction::Paste],
            FileCommand::Rename => &[WindowAction::Rename],
            FileCommand::Delete => &[WindowAction::Trash],
            FileCommand::DeletePermanently => &[WindowAction::DeletePermanently],
            FileCommand::Duplicate => &[WindowAction::Duplicate],
            FileCommand::Undo => &[WindowAction::Undo],
            FileCommand::Redo => &[WindowAction::Redo],
            FileCommand::Restore => &[WindowAction::Restore],
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
    /// It shows at least one item.
    pub(crate) has_items: bool,
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
    /// The desktop's clipboard holds files (`state.clipboard`).
    pub(crate) has_file_clipboard: bool,
    /// Undo has something to reverse.
    pub(crate) can_undo: bool,
    /// Redo has something to take back.
    pub(crate) can_redo: bool,
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
            FileCommand::New => !folder.is_searching && !self.is_busy && folder.is_writable,
            FileCommand::Copy => can_copy,
            FileCommand::Cut | FileCommand::Duplicate | FileCommand::Rename => {
                can_copy && !selection.has_read_only
            }
            FileCommand::Delete | FileCommand::DeletePermanently => can_change,
            FileCommand::Paste => {
                self.has_file_clipboard && !folder.is_searching && !busy && folder.is_writable
            }
            FileCommand::Undo => self.can_undo && !self.is_busy,
            FileCommand::Redo => self.can_redo && !self.is_busy,
            FileCommand::Restore => folder.is_recycle_bin && can_change,
            FileCommand::EmptyRecycleBin => folder.is_recycle_bin && folder.has_items && !self.is_busy,
            FileCommand::CancelOperation => self.is_busy,
        }
    }

    /// Why `command` is disabled, in the words a menu item's tooltip
    /// shows; `None` while it is enabled.
    pub(crate) fn refusal(&self, command: FileCommand) -> Option<&'static str> {
        if self.allows(command) {
            return None;
        }
        let selection = self.selection;
        let folder = self.folder;
        let reason = match command {
            FileCommand::CancelOperation => "No file operation is running.",
            FileCommand::Undo if !self.can_undo => "Nothing to undo.",
            FileCommand::Redo if !self.can_redo => "Nothing to redo.",
            FileCommand::Paste if !self.has_file_clipboard => "Nothing to paste here.",
            FileCommand::Restore if !folder.is_recycle_bin => {
                "Only items in the Recycle Bin can be restored."
            }
            FileCommand::EmptyRecycleBin if !folder.is_recycle_bin => "This is not the Recycle Bin.",
            FileCommand::EmptyRecycleBin if !folder.has_items => "The Recycle Bin is empty.",
            _ if self.is_busy => "Wait for the running file operation to finish.",
            FileCommand::New | FileCommand::Paste if folder.is_searching => {
                "Clear the search to add items to this folder."
            }
            FileCommand::New => "This folder is read-only.",
            FileCommand::Paste if !selection.has_inoperable => "This folder is read-only.",
            _ if selection.count == 0 => "Select an item first.",
            _ if selection.has_inoperable => "Drives, shares and virtual items cannot be changed here.",
            FileCommand::Copy | FileCommand::Cut | FileCommand::Rename | FileCommand::Duplicate
                if folder.is_recycle_bin =>
            {
                "Items in the Recycle Bin can only be restored or deleted."
            }
            _ if selection.has_read_only => "A previous version is read-only.",
            _ => "Rename one item at a time.",
        };
        Some(reason)
    }
}

/// The facts about `items`, the selected items (`canOperate` and
/// `readonlyLocation` in app.js).
pub(crate) fn selection_facts(items: &[FileItem], locations: &LocationContext) -> SelectionFacts {
    let has_inoperable = items.iter().any(|item| {
        let entry = item.entry();
        !entry.can_operate || entry.is_virtual || is_smb_share_root(&entry.uri)
    });
    let has_read_only = items
        .iter()
        .any(|item| locations.is_snapshot_location(&item.entry().uri));
    SelectionFacts {
        count: items.len(),
        has_inoperable,
        has_read_only,
    }
}

impl BrowserWindow {
    /// The facts the rules read now.
    pub(crate) fn command_facts(&self) -> CommandFacts {
        let model = self.folder_pane().model();
        let locations = self.imp().locations.borrow();
        let folder_uri = self.current_uri().unwrap_or_default();
        let folder = FolderFacts {
            is_writable: locations.is_writable_location(&folder_uri),
            is_searching: self.is_searching(),
            is_recycle_bin: same_location(&folder_uri, TRASH_URI),
            has_items: model.n_items() > 0,
        };
        let operations = self.imp().file_operations.borrow();
        let context = self.context();
        CommandFacts {
            selection: selection_facts(&model.selected_items(), &locations),
            folder,
            is_busy: operations.is_busy(),
            has_file_clipboard: operations.clipboard.is_some(),
            can_undo: context.journal_label(JournalDirection::Undo).is_some(),
            can_redo: context.journal_label(JournalDirection::Redo).is_some(),
        }
    }

    /// Whether `command` is enabled now.
    pub(crate) fn allows(&self, command: FileCommand) -> bool {
        self.command_facts().allows(command)
    }

    /// Enables and disables every file command, and labels Delete for the
    /// selection's folder (`updateToolbar`).
    pub(crate) fn update_file_commands(&self) {
        let facts = self.command_facts();
        for command in FileCommand::ALL {
            let enabled = facts.allows(command);
            for &action in command.actions() {
                self.set_action_enabled(action, enabled);
            }
        }
        self.command_bar().set_new_enabled(facts.allows(FileCommand::New));
        self.command_bar().show_delete_label(self.delete_label());
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
            has_file_clipboard: true,
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
                FileCommand::Rename,
                FileCommand::Delete,
                FileCommand::DeletePermanently,
                FileCommand::Duplicate,
            ]
        );
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
        // Several items are renamed together (OPS-014).
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

    /// parity: CMD-002, OPS-014
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

    #[test]
    fn paste_needs_files_on_the_clipboard() {
        let mut facts = selected(0);
        facts.has_file_clipboard = false;

        assert!(!facts.allows(FileCommand::Paste));
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
                FileCommand::EmptyRecycleBin,
            ]
        );
        assert!(!empty.allows(FileCommand::EmptyRecycleBin));
    }
}
