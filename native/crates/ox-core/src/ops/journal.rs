// SPDX-License-Identifier: AGPL-3.0-only
//! The undo journal: the operations Undo can reverse and the reversals
//! Redo can take back (OPS-029, OPS-031).
//!
//! New in the native app; the Python app has no Undo or Redo. Like the undo
//! managers of Dolphin and Nautilus, the journal keeps two stacks. Undo
//! takes the newest operation, and once its reversal ran, the record that
//! reverses the reversal ([`UndoRecord::inverse`]) goes onto the Redo stack;
//! Redo does the same the other way round. Each entry keeps the title of
//! the operation the user started, so both commands keep naming it ("Undo:
//! New folder", "Redo: New folder") whatever record reverses it now. A new
//! operation empties the Redo stack, as in every editor.

use std::collections::VecDeque;

use super::run_transfer::unix_seconds_now;
use super::undo::UndoRecord;

/// How many operations the journal remembers; older ones are dropped.
pub const UNDO_LIMIT: usize = 100;

/// Which way the journal is walked: back through the operations (Undo) or
/// forward through the undone ones (Redo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JournalDirection {
    /// Reverses the newest operation (Ctrl+Z).
    Undo,
    /// Takes back the newest Undo (Ctrl+Shift+Z, Ctrl+Y).
    Redo,
}

impl JournalDirection {
    /// The command's name, `Undo` or `Redo`.
    pub fn command_name(self) -> &'static str {
        match self {
            JournalDirection::Undo => crate::i18n::gettext_static("Undo"),
            JournalDirection::Redo => crate::i18n::gettext_static("Redo"),
        }
    }

    /// The other direction: a finished Undo can be redone, and a finished
    /// Redo undone.
    fn opposite(self) -> Self {
        match self {
            JournalDirection::Undo => JournalDirection::Redo,
            JournalDirection::Redo => JournalDirection::Undo,
        }
    }
}

/// One step either command can take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    /// The name of the operation the user started, such as `Rename`.
    pub title: &'static str,
    /// What reverses the operation's current effect.
    pub record: UndoRecord,
    /// When the step was recorded, in seconds since the Unix epoch, so
    /// Undo can tell which copies changed since (OPS-030).
    pub recorded_at: u64,
}

impl JournalEntry {
    /// The entry of a finished operation, titled after it.
    fn of_operation(record: UndoRecord) -> Self {
        Self {
            title: record.title(),
            record,
            recorded_at: unix_seconds_now(),
        }
    }

    /// The command label for this step, such as `Undo: Rename`.
    pub fn label(&self, direction: JournalDirection) -> String {
        crate::i18n::format_message(
            "{command}: {title}",
            &[("command", direction.command_name()), ("title", self.title)],
        )
    }
}

/// The steps Undo and Redo can take, newest last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UndoJournal {
    undo: VecDeque<JournalEntry>,
    redo: VecDeque<JournalEntry>,
}

impl UndoJournal {
    /// An empty journal: Undo and Redo are disabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Remembers a finished operation. A new operation makes the undone
    /// ones impossible to redo, so the Redo stack is emptied.
    pub fn record(&mut self, record: UndoRecord) {
        self.redo.clear();
        push_bounded(&mut self.undo, JournalEntry::of_operation(record));
    }

    /// The operation Undo would reverse next.
    pub fn last(&self) -> Option<&UndoRecord> {
        self.undo.back().map(|entry| &entry.record)
    }

    /// Removes and returns the operation to reverse now, forgetting its
    /// title; [`Self::take`] keeps it for Redo.
    pub fn take_last(&mut self) -> Option<UndoRecord> {
        self.undo.pop_back().map(|entry| entry.record)
    }

    /// True when there is nothing to undo, so the command is disabled.
    pub fn is_empty(&self) -> bool {
        self.undo.is_empty()
    }

    /// The number of operations that can be undone.
    pub fn len(&self) -> usize {
        self.undo.len()
    }

    /// The label of the command walking `direction`, such as `Undo:
    /// Rename`; `None` when it has nothing to do and is disabled.
    pub fn label(&self, direction: JournalDirection) -> Option<String> {
        let entry = self.stack(direction).back()?;
        Some(entry.label(direction))
    }

    /// Removes and returns the step `direction` takes now. Take it before
    /// the reversal starts, so no other window can take the same step;
    /// give it back with [`Self::put_back`] when the reversal changed
    /// nothing, or record what it did with [`Self::record_reversal`].
    pub fn take(&mut self, direction: JournalDirection) -> Option<JournalEntry> {
        self.stack_mut(direction).pop_back()
    }

    /// Returns a step taken in `direction` whose reversal was refused
    /// before it changed anything, so the command can try it again.
    pub fn put_back(&mut self, direction: JournalDirection, entry: JournalEntry) {
        push_bounded(self.stack_mut(direction), entry);
    }

    /// Records that the step titled `title` was reversed in `direction`:
    /// `inverse` (see [`UndoRecord::inverse`]) goes onto the other stack,
    /// so the other command can take the reversal back. Nothing is recorded
    /// when nothing was reversed.
    pub fn record_reversal(
        &mut self,
        direction: JournalDirection,
        title: &'static str,
        inverse: Option<UndoRecord>,
    ) {
        let Some(record) = inverse else {
            return;
        };
        let entry = JournalEntry {
            title,
            record,
            recorded_at: unix_seconds_now(),
        };
        push_bounded(self.stack_mut(direction.opposite()), entry);
    }

    fn stack(&self, direction: JournalDirection) -> &VecDeque<JournalEntry> {
        match direction {
            JournalDirection::Undo => &self.undo,
            JournalDirection::Redo => &self.redo,
        }
    }

    fn stack_mut(&mut self, direction: JournalDirection) -> &mut VecDeque<JournalEntry> {
        match direction {
            JournalDirection::Undo => &mut self.undo,
            JournalDirection::Redo => &mut self.redo,
        }
    }
}

/// Pushes `entry` onto `stack`, forgetting the oldest entry beyond
/// [`UNDO_LIMIT`].
fn push_bounded(stack: &mut VecDeque<JournalEntry>, entry: JournalEntry) {
    if stack.len() == UNDO_LIMIT {
        stack.pop_front();
    }
    stack.push_back(entry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::ItemKind;

    fn rename(number: usize) -> UndoRecord {
        UndoRecord::Rename {
            original_uri: format!("file:///tmp/{number}"),
            renamed_uri: format!("file:///tmp/{number}-renamed"),
        }
    }

    fn new_folder() -> UndoRecord {
        UndoRecord::Create {
            uri: "file:///tmp/New%20folder".into(),
            kind: ItemKind::Folder,
        }
    }

    /// What reversing the new folder's creation did: it went to the Trash,
    /// so the inverse restores it.
    fn restore_new_folder() -> UndoRecord {
        UndoRecord::Trash {
            original_paths: vec!["/tmp/New folder".into()],
            trashed_since: 7,
        }
    }

    #[test]
    fn undo_reverses_the_newest_operation_first() {
        let mut journal = UndoJournal::new();
        assert!(journal.is_empty());

        journal.record(rename(1));
        journal.record(rename(2));

        assert_eq!(journal.last(), Some(&rename(2)));
        assert_eq!(journal.take_last(), Some(rename(2)));
        assert_eq!(journal.take_last(), Some(rename(1)));
        assert_eq!(journal.take_last(), None);
    }

    #[test]
    fn the_journal_forgets_the_oldest_operation_beyond_its_limit() {
        let mut journal = UndoJournal::new();

        for number in 0..=UNDO_LIMIT {
            journal.record(rename(number));
        }

        assert_eq!(journal.len(), UNDO_LIMIT);
        let oldest = std::iter::from_fn(|| journal.take_last()).last();
        assert_eq!(oldest, Some(rename(1)));
    }

    /// parity: OPS-029, OPS-031
    #[test]
    fn undo_and_redo_keep_naming_the_operation_the_user_started() {
        let mut journal = UndoJournal::new();
        journal.record(new_folder());
        assert_eq!(
            journal.label(JournalDirection::Undo).as_deref(),
            Some("Undo: New folder")
        );
        assert_eq!(journal.label(JournalDirection::Redo), None);

        let undone = journal
            .take(JournalDirection::Undo)
            .expect("an operation to undo");
        journal.record_reversal(JournalDirection::Undo, undone.title, Some(restore_new_folder()));

        assert_eq!(journal.label(JournalDirection::Undo), None);
        assert_eq!(
            journal.label(JournalDirection::Redo).as_deref(),
            Some("Redo: New folder")
        );
        let redone = journal.take(JournalDirection::Redo).expect("an undo to redo");
        assert_eq!(redone.record, restore_new_folder());
        let restored = UndoRecord::Restore {
            restored: vec!["file:///tmp/New%20folder".into()],
        };
        journal.record_reversal(JournalDirection::Redo, redone.title, Some(restored));
        assert_eq!(
            journal.label(JournalDirection::Undo).as_deref(),
            Some("Undo: New folder")
        );
    }

    #[test]
    fn a_new_operation_cannot_be_followed_by_an_older_redo() {
        let mut journal = UndoJournal::new();
        journal.record(rename(1));
        let undone = journal
            .take(JournalDirection::Undo)
            .expect("an operation to undo");
        journal.record_reversal(JournalDirection::Undo, undone.title, Some(rename(9)));

        journal.record(rename(2));

        assert_eq!(journal.label(JournalDirection::Redo), None);
        assert_eq!(
            journal.label(JournalDirection::Undo).as_deref(),
            Some("Undo: Rename")
        );
    }

    #[test]
    fn a_refused_reversal_is_put_back_and_a_reversal_of_nothing_is_not_recorded() {
        let mut journal = UndoJournal::new();
        journal.record(rename(1));

        let refused = journal
            .take(JournalDirection::Undo)
            .expect("an operation to undo");
        journal.put_back(JournalDirection::Undo, refused);
        let nothing_done = journal.take(JournalDirection::Undo).expect("the step is back");
        journal.record_reversal(JournalDirection::Undo, nothing_done.title, None);

        assert!(journal.is_empty());
        assert_eq!(journal.label(JournalDirection::Redo), None);
    }
}
