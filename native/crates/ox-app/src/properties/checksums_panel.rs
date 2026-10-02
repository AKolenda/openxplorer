// SPDX-License-Identifier: AGPL-3.0-only
//! The Checksums tab of a file's Properties (PROP-014), after Dolphin's:
//! MD5, SHA1, SHA256 and SHA512, each computed in the background when
//! asked for and then copied with one click, and a field where a pasted
//! checksum is checked against the algorithm its length names.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use ox_core::checksums::{compute_in_background, matches, ChecksumKind};
use ox_core::entry::EntryError;
use ox_core::transfer::Cancellation;

use super::general_panel::glyph_button;
use super::CALCULATING;
use crate::dialog::{quiet_text, PropertyGrid};
use crate::icons::Icon;

/// A checksum not asked for yet.
const NOT_CALCULATED: &str = crate::i18n::message_id("Not calculated");
/// The verdict when the pasted checksum is the file's.
pub(super) const MATCH: &str = crate::i18n::message_id("Checksums match.");
/// The verdict when it is not.
pub(super) const MISMATCH: &str = crate::i18n::message_id("Checksums do not match.");
/// The verdict for text that is no known checksum.
const NOT_A_CHECKSUM: &str = crate::i18n::message_id("Enter an MD5, SHA1, SHA256 or SHA512 checksum.");
/// Above the field for the expected checksum.
const VERIFY_NOTE: &str = crate::i18n::message_id("Paste the checksum the file should have to check it.");
/// The toast after a checksum was copied.
const COPIED: &str = crate::i18n::message_id("Checksum copied.");

/// One algorithm's line: its value and its Calculate or Copy button.
#[derive(Debug)]
struct Row {
    value: gtk::Label,
    button: gtk::Button,
}

/// What the tab knows and shows.
#[derive(Debug)]
struct Checksums {
    uri: String,
    /// Stops every computation when the dialog closes.
    cancel: Cancellation,
    computed: RefCell<HashMap<ChecksumKind, String>>,
    rows: HashMap<ChecksumKind, Row>,
    expected: gtk::Entry,
    verdict: gtk::Label,
}

/// The Checksums tab; it keeps its state while the dialog is open.
#[derive(Debug)]
pub(crate) struct ChecksumsPanel {
    widget: gtk::Box,
    checksums: Rc<Checksums>,
}

impl ChecksumsPanel {
    /// The tab for the file at `uri`.
    pub(super) fn new(uri: &str) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let grid = PropertyGrid::new();
        let mut rows = HashMap::new();
        for (line, kind) in (0..).zip(ChecksumKind::ALL) {
            let value = grid.add_row(kind.label(), ox_core::i18n::gettext_static(NOT_CALCULATED));
            value.set_wrap_mode(gtk::pango::WrapMode::Char);
            let button = glyph_button("Calculate", Icon::Checkmark);
            button.set_valign(gtk::Align::Start);
            let name =
                ox_core::i18n::format_message("Calculate {label}", &[("label", &(kind.label()).to_string())]);
            button.update_property(&[gtk::accessible::Property::Label(&name)]);
            grid.widget().attach(&button, 2, line, 1, 1);
            rows.insert(kind, Row { value, button });
        }
        widget.append(grid.widget());
        let expected = gtk::Entry::builder()
            .placeholder_text(ox_core::i18n::gettext("Expected checksum"))
            .build();
        expected.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
            "Expected checksum",
        ))]);
        let verdict = quiet_text("");
        let checksums = Rc::new(Checksums {
            uri: uri.to_owned(),
            cancel: Cancellation::new(),
            computed: RefCell::default(),
            rows,
            expected,
            verdict,
        });
        for kind in ChecksumKind::ALL {
            checksums.rows[&kind].button.connect_clicked(glib::clone!(
                #[weak]
                checksums,
                move |button| checksums.calculate_or_copy(kind, button)
            ));
        }
        widget.append(&quiet_text(ox_core::i18n::gettext_static(VERIFY_NOTE)));
        widget.append(&checksums.expected);
        widget.append(&checksums.verdict);
        checksums.expected.connect_changed(glib::clone!(
            #[weak]
            checksums,
            move |_| checksums.verify()
        ));
        Self { widget, checksums }
    }

    /// The tab's widget.
    pub(super) fn widget(&self) -> &gtk::Box {
        &self.widget
    }

    /// Stops the computations in progress.
    pub(super) fn cancel(&self) {
        self.checksums.cancel.cancel();
    }

    /// The value shown for `kind`, for tests.
    #[cfg(test)]
    pub(crate) fn value_text(&self, kind: ChecksumKind) -> String {
        self.checksums.rows[&kind].value.text().to_string()
    }

    /// Pastes `text` as the expected checksum, for tests.
    #[cfg(test)]
    pub(crate) fn paste_expected(&self, text: &str) {
        self.checksums.expected.set_text(text);
    }

    /// The verdict shown, for tests.
    #[cfg(test)]
    pub(crate) fn verdict_text(&self) -> String {
        self.checksums.verdict.text().to_string()
    }
}

impl Checksums {
    /// Calculate: computes `kind`; once computed, the button copies it.
    fn calculate_or_copy(self: &Rc<Self>, kind: ChecksumKind, button: &gtk::Button) {
        let computed = self.computed.borrow().get(&kind).cloned();
        match computed {
            Some(value) => {
                button.clipboard().set_text(&value);
                if let Some(window) = button.root().and_downcast::<crate::window::BrowserWindow>() {
                    window.show_message(ox_core::i18n::gettext_static(COPIED));
                }
            }
            None => self.calculate(kind),
        }
    }

    /// Computes `kind` in the background unless it is computed or being
    /// computed.
    fn calculate(self: &Rc<Self>, kind: ChecksumKind) {
        let row = &self.rows[&kind];
        if self.computed.borrow().contains_key(&kind) || row.value.text() == CALCULATING {
            return;
        }
        row.value.set_text(CALCULATING);
        row.button.set_sensitive(false);
        let computing = compute_in_background(self.uri.clone(), kind, self.cancel.clone());
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = checksums)]
            self,
            async move {
                let result = computing.await;
                checksums.show(kind, result);
            }
        ));
    }

    /// Shows the outcome of computing `kind`.
    fn show(self: &Rc<Self>, kind: ChecksumKind, result: Result<String, EntryError>) {
        let row = &self.rows[&kind];
        row.button.set_sensitive(true);
        match result {
            Ok(value) => {
                row.value.set_text(&value);
                set_button_label(
                    &row.button,
                    "Copy",
                    &ox_core::i18n::format_message("Copy {label}", &[("label", &(kind.label()).to_string())]),
                );
                self.computed.borrow_mut().insert(kind, value);
            }
            Err(EntryError::Cancelled) => row.value.set_text(ox_core::i18n::gettext_static(NOT_CALCULATED)),
            Err(error) => row.value.set_text(&error.to_string()),
        }
        self.verify();
    }

    /// Checks the pasted checksum, computing its algorithm first.
    fn verify(self: &Rc<Self>) {
        let text = self.expected.text();
        if text.trim().is_empty() {
            self.verdict.set_text("");
            return;
        }
        let Some(kind) = ChecksumKind::of_expected(&text) else {
            self.verdict
                .set_text(ox_core::i18n::gettext_static(NOT_A_CHECKSUM));
            return;
        };
        let computed = self.computed.borrow().get(&kind).cloned();
        match computed {
            Some(value) if matches(&text, &value) => {
                self.verdict.set_text(ox_core::i18n::gettext_static(MATCH))
            }
            Some(_) => self.verdict.set_text(ox_core::i18n::gettext_static(MISMATCH)),
            None => {
                self.verdict.set_text(CALCULATING);
                self.calculate(kind);
            }
        }
    }
}

/// Replaces the text of a glyph button made by [`glyph_button`], and its
/// accessible name.
fn set_button_label(button: &gtk::Button, label: &str, name: &str) {
    let text = button
        .child()
        .and_then(|content| content.last_child())
        .and_downcast::<gtk::Label>();
    if let Some(text) = text {
        text.set_text(label);
    }
    button.update_property(&[gtk::accessible::Property::Label(name)]);
}
