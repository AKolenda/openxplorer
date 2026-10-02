// SPDX-License-Identifier: AGPL-3.0-only
//! The parts dialog bodies are built from: labelled text fields, notes,
//! check boxes and key-value grids.
//!
//! Ports `textField`, `propertyRow` and the `.modal-note`, `.hint`,
//! `.checkbox-row` and `.property-grid` markup of `v2.0.0:desktop/ui/app.js`;
//! `resources/skin/in-window-dialogs.css` draws them.

use std::cell::Cell;

use gtk::prelude::*;

/// Shown for a value the backend did not report (`propertyRow`).
pub(crate) const NOT_PROVIDED: &str = crate::i18n::message_id("Not provided");

/// Adds a text field labelled `label` holding `text` to `body`, and
/// returns the field. The label names the field for screen readers too.
pub(crate) fn labelled_entry(body: &gtk::Box, label: &str, text: &str) -> gtk::Entry {
    let entry = gtk::Entry::builder().text(text).hexpand(true).build();
    body.append(&field_caption(label, &entry));
    body.append(&entry);
    entry
}

/// The label over `control` (`label.field-label`), which names it for
/// screen readers too and moves focus to it with its mnemonic.
pub(super) fn field_caption(label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Label {
    let caption = gtk::Label::builder()
        .label(label)
        .xalign(0.0)
        .css_classes(["field-label"])
        .mnemonic_widget(control)
        .build();
    let control = control.upcast_ref::<gtk::Widget>();
    control.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
    caption
}

/// A boxed note in the muted colour (`.modal-note`).
pub(crate) fn note(text: &str) -> gtk::Label {
    wrapped_label(text, "modal-note")
}

/// A paragraph of muted text (`.quiet`, `.hint`, `p`).
pub(crate) fn quiet_text(text: &str) -> gtk::Label {
    wrapped_label(text, "quiet")
}

/// A left-aligned label that wraps, with the class `class`.
pub(super) fn wrapped_label(text: &str, class: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes([class])
        .build()
}

/// A check box labelled `label` (`.checkbox-row`).
pub(crate) fn check_row(label: &str, is_active: bool) -> gtk::CheckButton {
    check_box(label, is_active, "checkbox-row")
}

/// A check box labelled `label` with the class `class`.
pub(super) fn check_box(label: &str, is_active: bool, class: &str) -> gtk::CheckButton {
    gtk::CheckButton::builder()
        .label(label)
        .active(is_active)
        .css_classes([class])
        .build()
}

/// A grid of names and values (`dl.property-grid`): names in the muted
/// colour, values selectable so they can be copied.
#[derive(Debug)]
pub(crate) struct PropertyGrid {
    grid: gtk::Grid,
    /// The rows added so far; the next row goes at this index.
    rows: Cell<i32>,
}

impl PropertyGrid {
    /// An empty grid.
    pub(crate) fn new() -> Self {
        Self {
            grid: gtk::Grid::builder().css_classes(["property-grid"]).build(),
            rows: Cell::new(0),
        }
    }

    /// The grid widget, to add to a panel.
    pub(crate) fn widget(&self) -> &gtk::Grid {
        &self.grid
    }

    /// Adds the row `name` with `value`, or [`NOT_PROVIDED`] for an empty
    /// value, and returns the value's label so it can change later.
    pub(crate) fn add_row(&self, name: &str, value: &str) -> gtk::Label {
        let row = self.rows.get();
        let name_label = gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .yalign(0.0)
            .css_classes(["property-name"])
            .build();
        let value_label = gtk::Label::builder()
            .label(value_or_not_provided(value))
            .xalign(0.0)
            .hexpand(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .selectable(true)
            .css_classes(["property-value"])
            .build();
        value_label.update_relation(&[gtk::accessible::Relation::LabelledBy(&[name_label.upcast_ref()])]);
        self.grid.attach(&name_label, 0, row, 1, 1);
        self.grid.attach(&value_label, 1, row, 1, 1);
        self.rows.set(row + 1);
        value_label
    }
}

/// `value`, or [`NOT_PROVIDED`] when it is empty, as `propertyRow` shows
/// a missing value.
fn value_or_not_provided(value: &str) -> &str {
    if value.is_empty() {
        ox_core::i18n::gettext_static(NOT_PROVIDED)
    } else {
        value
    }
}
