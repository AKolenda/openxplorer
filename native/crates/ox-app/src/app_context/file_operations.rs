// SPDX-License-Identifier: AGPL-3.0-only
//! What the file operations of every window share: the undo journal
//! (OPS-029, OPS-031). Their previous-versions protection (XFER-020) is
//! [`super::previous_versions`]'s.
//!
//! The Python app had no Undo. Like Dolphin's undo manager, the journal
//! here belongs to the application: Undo in any window reverses the newest
//! operation of any window, and every window hears `journal-changed` to
//! relabel its Undo and Redo commands.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::ops::{JournalDirection, JournalEntry, UndoRecord};

use super::{AppContext, JOURNAL_CHANGED};

impl AppContext {
    /// The label of the Undo or Redo command, such as `Undo: Rename`;
    /// `None` while it has nothing to do.
    pub(crate) fn journal_label(&self, direction: JournalDirection) -> Option<String> {
        self.imp().undo_journal.borrow().label(direction)
    }

    /// Remembers a finished operation for Undo.
    pub(crate) fn record_operation(&self, record: UndoRecord) {
        self.imp().undo_journal.borrow_mut().record(record);
        self.notify_journal_changed();
    }

    /// Takes the step Undo or Redo carries out now, so no other window
    /// takes it too.
    pub(crate) fn take_journal_step(&self, direction: JournalDirection) -> Option<JournalEntry> {
        let step = self.imp().undo_journal.borrow_mut().take(direction);
        self.notify_journal_changed();
        step
    }

    /// Returns a step whose reversal was refused before anything changed.
    pub(crate) fn put_back_journal_step(&self, direction: JournalDirection, step: JournalEntry) {
        self.imp().undo_journal.borrow_mut().put_back(direction, step);
        self.notify_journal_changed();
    }

    /// Records what a reversal in `direction` did, so the other command
    /// can take it back.
    pub(crate) fn record_reversal(
        &self,
        direction: JournalDirection,
        title: &'static str,
        inverse: Option<UndoRecord>,
    ) {
        let mut journal = self.imp().undo_journal.borrow_mut();
        journal.record_reversal(direction, title, inverse);
        drop(journal);
        self.notify_journal_changed();
    }

    fn notify_journal_changed(&self) {
        self.emit_by_name::<()>(JOURNAL_CHANGED, &[]);
    }

    /// Calls `callback` whenever Undo or Redo would now do something else.
    pub(crate) fn connect_journal_changed(&self, callback: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(JOURNAL_CHANGED, false, move |_| {
            callback();
            None
        })
    }
}
