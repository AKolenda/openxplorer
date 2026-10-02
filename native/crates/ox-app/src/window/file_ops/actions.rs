// SPDX-License-Identifier: AGPL-3.0-only
//! The window actions of the file commands, and what keeps their enabled
//! state current.
//!
//! Each command that asks or waits runs as a task on the main loop, so a
//! dialog or a running operation never blocks the window. The commands
//! are enabled by [`super::availability`], which runs again whenever the
//! selection, the folder, the clipboard, the undo journal or the running
//! operation changes.

use std::future::Future;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::clipboard::ClipboardMode;
use ox_core::ops::{BuiltinTemplate, JournalDirection};

use super::new_items::NewFileKind;
use crate::window::actions::plain_action;
use crate::window::window_action::WindowAction;
use crate::window::BrowserWindow;

/// An action that runs `task` on the main loop.
fn task_action<Task>(
    window_action: WindowAction,
    task: impl Fn(BrowserWindow) -> Task + 'static,
) -> gio::ActionEntry<BrowserWindow>
where
    Task: Future<Output = ()> + 'static,
{
    plain_action(window_action, move |window| {
        glib::spawn_future_local(task(window.clone()));
    })
}

/// A New menu item that opens the template dialog for `kind`.
fn new_file_action(window_action: WindowAction, kind: NewFileKind) -> gio::ActionEntry<BrowserWindow> {
    task_action(window_action, move |window| async move {
        window.create_file(kind).await;
    })
}

impl BrowserWindow {
    /// Adds the file commands as window actions and their keys, and keeps
    /// their enabled state current. Returns the handlers on the undo
    /// journal and the clipboard, which outlive the window.
    pub(crate) fn install_file_actions(&self) -> [glib::SignalHandlerId; 2] {
        self.install_new_actions();
        self.install_edit_actions();
        self.install_operation_actions();
        self.install_file_shortcuts();
        let journal = self.context().connect_journal_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                window.withdraw_toast_undo();
                window.update_file_commands();
            }
        ));
        let clipboard = self.follow_file_clipboard();
        self.update_file_commands();
        [journal, clipboard]
    }

    /// New ▸ Folder, the New menu's files and New ▸ Link.
    fn install_new_actions(&self) {
        let starter = NewFileKind::Starter;
        self.add_action_entries([
            task_action(WindowAction::NewFolder, |window| async move {
                window.create_folder().await;
            }),
            new_file_action(WindowAction::NewTextDocument, starter(BuiltinTemplate::Text)),
            new_file_action(WindowAction::NewFile, NewFileKind::Empty),
            new_file_action(
                WindowAction::NewMarkdownDocument,
                starter(BuiltinTemplate::Markdown),
            ),
            new_file_action(WindowAction::NewCsvFile, starter(BuiltinTemplate::Csv)),
            new_file_action(WindowAction::NewJsonFile, starter(BuiltinTemplate::Json)),
            new_file_action(WindowAction::NewHtmlDocument, starter(BuiltinTemplate::Html)),
            new_file_action(WindowAction::NewFromTemplate, NewFileKind::AnyTemplate),
            task_action(WindowAction::NewLink, |window| async move {
                window.create_link().await;
            }),
        ]);
    }

    /// Cut, Copy, Paste, Rename, Delete, Shift+Delete and Duplicate.
    fn install_edit_actions(&self) {
        self.add_action_entries([
            plain_action(WindowAction::Cut, |window| {
                window.copy_selection(ClipboardMode::Cut);
            }),
            plain_action(WindowAction::Copy, |window| {
                window.copy_selection(ClipboardMode::Copy);
            }),
            task_action(WindowAction::Paste, |window| async move {
                window.paste().await;
            }),
            task_action(WindowAction::Rename, |window| async move {
                window.rename_selection().await;
            }),
            task_action(WindowAction::Trash, |window| async move {
                window.delete_selection().await;
            }),
            task_action(WindowAction::DeletePermanently, |window| async move {
                window.delete_selection_permanently().await;
            }),
            task_action(WindowAction::Duplicate, |window| async move {
                window.duplicate_selection().await;
            }),
        ]);
    }

    /// Undo, Redo, Cancel and the Recycle Bin's commands.
    fn install_operation_actions(&self) {
        self.add_action_entries([
            task_action(WindowAction::Undo, |window| async move {
                window.walk_journal(JournalDirection::Undo).await;
            }),
            task_action(WindowAction::Redo, |window| async move {
                window.walk_journal(JournalDirection::Redo).await;
            }),
            plain_action(WindowAction::CancelOperation, BrowserWindow::cancel_operation),
            task_action(WindowAction::Restore, |window| async move {
                window.restore_selected_items().await;
            }),
            task_action(WindowAction::EmptyRecycleBin, |window| async move {
                window.empty_recycle_bin().await;
            }),
            task_action(WindowAction::EmptyTrash, |window| async move {
                window.empty_trash().await;
            }),
            plain_action(WindowAction::ClearRecentFiles, |window| {
                window.context().clear_recent_files();
            }),
        ]);
    }
}
