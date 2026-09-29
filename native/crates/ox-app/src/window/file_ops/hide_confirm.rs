// SPDX-License-Identifier: AGPL-3.0-only
//! Asking before a rename hides an item (OPS-013).
//!
//! Dolphin asks "Adding a dot to the beginning of this file's name will
//! hide it from view." when a visible item gets a name starting with a
//! dot while hidden files are not shown, so the item does not seem to
//! vanish. "Don't ask again" stops the question for the rest of the
//! window's session.

use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::window::dialog::{ButtonStyle, Dialog};
use crate::window::BrowserWindow;

/// The question's title.
const TITLE: &str = "Rename and hide?";

/// The question, in Dolphin's words.
const MESSAGE: &str = "Adding a dot to the beginning of this file's name will hide it from view.";

/// True when renaming `old_name` to `new_name` hides a visible item while
/// hidden files are not shown (`hidden_shown` false).
fn would_hide(old_name: &str, new_name: &str, hidden_shown: bool) -> bool {
    !hidden_shown && new_name.starts_with('.') && !old_name.starts_with('.')
}

impl BrowserWindow {
    /// True when the rename of `old_name` to `new_name` may go ahead:
    /// it hides nothing, or the user confirmed "Rename and Hide".
    pub(super) async fn confirm_hiding_rename(&self, old_name: &str, new_name: &str) -> bool {
        let asks = !self.imp().file_operations.borrow().hiding_confirmed;
        if !asks || !would_hide(old_name, new_name, self.hidden_files_shown()) {
            return true;
        }
        let dialog = Dialog::new(self, TITLE, MESSAGE);
        let dont_ask = dialog.add_check_button("Don't ask again", false);
        dialog.add_cancel_button();
        dialog.add_button("Rename and Hide", ButtonStyle::Primary);
        dialog.open();
        if dialog.next_response().await.is_none() {
            return false;
        }
        if dont_ask.is_active() {
            self.imp().file_operations.borrow_mut().hiding_confirmed = true;
        }
        dialog.finish();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: OPS-013
    #[test]
    fn only_a_new_leading_dot_with_hidden_files_off_asks() {
        assert!(would_hide("notes.txt", ".notes.txt", false));
        assert!(!would_hide("notes.txt", ".notes.txt", true), "hidden files are shown");
        assert!(!would_hide(".notes", ".old-notes", false), "it was hidden already");
        assert!(!would_hide("notes.txt", "notes.md", false));
    }
}
