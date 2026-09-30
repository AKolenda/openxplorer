// SPDX-License-Identifier: AGPL-3.0-only
//! Batch rename: F2 with several items selected (OPS-014).
//!
//! New in the native app, from Dolphin's Rename Items dialog: "Rename the
//! <n> selected items to:", a name that starts as "New name #", and the
//! number `#` starts at. Each item keeps its extension, and a name
//! without `#` is accepted only when the items' types differ
//! ([`BatchRename::new_names`]); a refused name stays in the dialog. The
//! renames run as the window's one operation, the renamed items are
//! selected afterwards, and Undo renames the whole batch back (OPS-029).

use gtk::prelude::*;
use ox_core::entry::Entry;
use ox_core::ops::{
    rename_batch, summarize_batch_rename, BatchItem, BatchRename, DEFAULT_BATCH_NAME, NUMBER_PLACEHOLDER,
};

use super::running::FinishedOperation;
use crate::window::dialog::{ButtonStyle, Dialog};
use crate::window::BrowserWindow;

/// The dialog's title (Dolphin's "Rename Items").
const TITLE: &str = "Rename items";

/// The label of the first number's field.
const FIRST_NUMBER_LABEL: &str = "# becomes ascending numbers starting at";

/// The panel's label while the items are renamed.
const RENAMING: &str = "Renaming items…";

/// The highest first number the field takes.
const MAX_FIRST_NUMBER: f64 = 999_999.0;

impl BrowserWindow {
    /// Asks for the batch's name and first number, then renames `entries`
    /// in the order they are shown.
    pub(super) async fn rename_several(&self, entries: &[Entry]) {
        let items: Vec<BatchItem> = entries
            .iter()
            .map(|entry| BatchItem {
                uri: entry.uri.clone(),
                name: entry.name.clone(),
                is_dir: entry.is_dir,
            })
            .collect();
        let message = format!("Rename the {} selected items to:", items.len());
        let dialog = Dialog::new(self, TITLE, &message);
        let name = dialog.add_text_field("Name", DEFAULT_BATCH_NAME);
        let first_number = gtk::SpinButton::with_range(0.0, MAX_FIRST_NUMBER, 1.0);
        first_number.set_value(1.0);
        first_number.set_activates_default(true);
        dialog.add_labelled(FIRST_NUMBER_LABEL, &first_number);
        dialog.add_cancel_button();
        dialog.add_button("Rename", ButtonStyle::Primary);
        dialog.open();
        select_before_number(&name);
        let batch = loop {
            if dialog.next_response().await.is_none() {
                return;
            }
            let batch = BatchRename {
                items: items.clone(),
                pattern: name.text().to_string(),
                first_number: u32::try_from(first_number.value_as_int()).unwrap_or(0),
            };
            match batch.new_names() {
                Ok(_) => break batch,
                Err(refusal) => dialog.show_error(&refusal.to_string()),
            }
        };
        dialog.finish();
        self.run_batch_rename(&batch).await;
    }

    /// Renames `batch` as the window's one operation and reports it.
    async fn run_batch_rename(&self, batch: &BatchRename) {
        let Some(context) = self.begin_operation(RENAMING) else {
            return;
        };
        let outcome = rename_batch(batch, &context).await;
        self.end_operation();
        let finished = outcome.map(|outcome| FinishedOperation {
            summary: summarize_batch_rename(&outcome.result),
            undo: outcome.undo,
            select_after: outcome.created,
        });
        self.conclude_operation(finished).await;
    }
}

/// Selects the name before its number, "New name " at first, so typing
/// replaces it and keeps the number.
fn select_before_number(field: &gtk::Entry) {
    let text = field.text();
    let end = text
        .find(NUMBER_PLACEHOLDER)
        .map_or_else(|| text.chars().count(), |index| text[..index].chars().count());
    field.select_region(0, i32::try_from(end).unwrap_or(-1));
}
