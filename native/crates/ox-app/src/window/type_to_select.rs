// SPDX-License-Identifier: AGPL-3.0-only
//! Type-to-select in the folder views: the typed prefix, the timer that
//! ends it and its hint in the status bar.
//!
//! Ports the type-select glue in `v2.0.0:desktop/ui/app.js`
//! (`v2.0.0:desktop/tests/ui_type_select.py` is its specification): typed
//! characters jump to the next name with that prefix, Backspace shortens
//! it, and a pause ends it. [`crate::typeahead`] holds the matching rules;
//! [`super::input`] decides which keys and clicks reach them.

use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use crate::folder_view::model::FolderModel;
use crate::typeahead::{self, PrefixMatch, Rows};

use super::status_bar::TypeaheadMatch;
use super::widget_tree::descendants;
use super::BrowserWindow;

/// The type-to-select prefix and the timer that clears its hint.
#[derive(Debug, Default)]
pub(super) struct Typeahead {
    /// The typed prefix and the matching rules.
    controller: typeahead::Controller,
    /// Ends the prefix after a pause; it clears itself when it fires.
    timer: Option<glib::SourceId>,
}

impl Typeahead {
    /// Whether a typed prefix is still in effect at `now`.
    pub(super) fn is_active(&self, now: Duration) -> bool {
        self.controller.is_active(now)
    }

    /// Stops the timer that would end the prefix, when it runs.
    pub(super) fn stop_timer(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
    }
}

/// The status-bar hint for a type-to-select result.
fn typeahead_hint(result: &PrefixMatch, matched_name: Option<&str>) -> String {
    match (result.prefix.is_empty(), matched_name) {
        (true, _) => String::new(),
        (false, Some(name)) => ox_core::i18n::format_message(
            "Jump to: {prefix} — {name}",
            &[("prefix", &result.prefix), ("name", name)],
        ),
        (false, None) => {
            ox_core::i18n::format_message("No name starts with “{prefix}”", &[("prefix", &result.prefix)])
        }
    }
}

/// The current time on `GLib`'s monotonic clock, which type-to-select
/// times its prefix with. The clock never reads below zero and never goes
/// backwards.
pub(super) fn monotonic_now() -> Duration {
    Duration::from_micros(glib::monotonic_time().unsigned_abs())
}

/// The folder's rows in display order, as type-to-select searches them.
fn typeahead_rows(model: &FolderModel) -> Rows<impl Fn(u32) -> String + '_> {
    Rows {
        count: model.n_items(),
        name_at: |row| model.name_at(row).unwrap_or_default(),
        current: model.first_selected(),
    }
}

impl BrowserWindow {
    /// Backspace: removes the last typed character and selects what the
    /// shorter prefix matches.
    pub(super) fn erase_typed_character(&self, now: Duration) {
        let rows = typeahead_rows(self.folder_pane().model());
        let result = self.imp().typeahead.borrow_mut().controller.backspace(&rows, now);
        if let Some(result) = result {
            self.apply_typeahead(&result);
        }
    }

    /// Adds text the input method committed to the typed prefix and
    /// selects the next matching name.
    pub(super) fn type_text(&self, text: &str) {
        for character in text.chars() {
            self.type_character(character);
        }
    }

    /// Adds one typed character to the prefix. Each character moves the
    /// selection the next one starts from, so the rows are read again.
    fn type_character(&self, character: char) {
        let rows = typeahead_rows(self.folder_pane().model());
        let typed = character.to_string();
        let result = self
            .imp()
            .typeahead
            .borrow_mut()
            .controller
            .push(&typed, &rows, monotonic_now());
        if let Some(result) = result {
            self.apply_typeahead(&result);
        }
    }

    fn apply_typeahead(&self, result: &PrefixMatch) {
        let model = self.folder_pane().model();
        if let Some(row) = result.row {
            // Through the view, so the match becomes the range anchor.
            self.folder_pane().select_and_reveal(row);
        }
        let matched_name = result.row.and_then(|row| model.name_at(row));
        let hint = typeahead_hint(result, matched_name.as_deref());
        let outcome = if result.row.is_some() {
            TypeaheadMatch::Found
        } else {
            TypeaheadMatch::Missed
        };
        self.status_bar().show_typeahead_hint(&hint, outcome);
        self.restart_typeahead_timer();
    }

    fn restart_typeahead_timer(&self) {
        let timer = glib::timeout_add_local_once(
            typeahead::PREFIX_TIMEOUT,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || {
                    window.imp().typeahead.borrow_mut().timer = None;
                    window.reset_typeahead();
                }
            ),
        );
        let previous = self.imp().typeahead.borrow_mut().timer.replace(timer);
        if let Some(previous) = previous {
            previous.remove();
        }
    }

    /// Forgets the typed prefix and clears its hint.
    pub(super) fn reset_typeahead(&self) {
        let timer = {
            let mut typeahead = self.imp().typeahead.borrow_mut();
            typeahead.controller.reset();
            typeahead.timer.take()
        };
        if let Some(timer) = timer {
            timer.remove();
        }
        self.status_bar().clear_typeahead_hint();
    }

    /// Ends the type-to-select prefix and closes every open menu of the
    /// window, as opening a dialog does in app.js.
    pub(crate) fn quiet_for_dialog(&self) {
        self.reset_typeahead();
        let popovers = descendants::<gtk::Popover>(self);
        for popover in popovers.iter().filter(|popover| popover.is_visible()) {
            popover.popdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(prefix: &str, row: Option<u32>) -> PrefixMatch {
        PrefixMatch {
            prefix: prefix.into(),
            row,
            cycling: false,
        }
    }

    /// Ported from `v2.0.0:desktop/tests/ui_type_select.py` (the "Jump to" hint).
    #[test]
    fn the_hint_names_the_item_it_jumped_to() {
        let hint = typeahead_hint(&result("SC", Some(3)), Some("scripts"));
        assert_eq!(hint, "Jump to: SC — scripts");
        let miss = typeahead_hint(&result("zz", None), None);
        assert_eq!(miss, "No name starts with “zz”");
        assert_eq!(typeahead_hint(&result("", None), None), "");
    }
}
