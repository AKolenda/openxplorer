// SPDX-License-Identifier: AGPL-3.0-only
//! The label under the drag icon that says what releasing the drag would
//! do, as Windows 11 File Explorer's drop description does (DND-028):
//! "→ Move to Documents", "+ Copy to Documents" or "Create link in
//! Documents", with the arrow, plus or link glyph before the text.
//!
//! Only a drag of this process carries the label, so only this app's drop
//! targets can show it, in any of its windows: they say what a release
//! would do with [`show_drag_label`], and hide it where nothing takes the
//! drop or what the drop does is not one of the three. The label is part
//! of the drag icon, so it follows the pointer outside the windows too,
//! hidden there.

use std::cell::RefCell;

use gtk::glib;
use gtk::prelude::*;

use crate::icons::{self, Icon};

/// The edge of the glyph before the text.
const GLYPH_SIZE: i32 = 12;

/// What a release would do with the dragged items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DragVerb {
    /// Moves them into the place.
    Move,
    /// Copies them into the place.
    Copy,
    /// Makes links to them in the place.
    Link,
}

/// What the label says: what a release would do, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DragLabel {
    /// What a release would do.
    pub(crate) verb: DragVerb,
    /// The name of the folder or drive the items would go into.
    pub(crate) place: String,
}

impl DragLabel {
    /// The label's text, in Windows' words.
    pub(crate) fn text(&self) -> String {
        let place = [("place", self.place.as_str())];
        match self.verb {
            DragVerb::Move => ox_core::i18n::format_message("Move to {place}", &place),
            DragVerb::Copy => ox_core::i18n::format_message("Copy to {place}", &place),
            DragVerb::Link => ox_core::i18n::format_message("Create link in {place}", &place),
        }
    }

    /// The glyph before the text: Windows' arrow, plus or link.
    fn glyph(&self) -> Icon {
        match self.verb {
            DragVerb::Move => Icon::ArrowRight,
            DragVerb::Copy => Icon::Add,
            DragVerb::Link => Icon::Link,
        }
    }
}

/// The parts of the label on the icon of the drag this process makes now.
#[derive(Debug)]
struct ShownLabel {
    /// The label, hidden while there is nothing to say.
    row: glib::WeakRef<gtk::Box>,
    /// Its glyph.
    glyph: glib::WeakRef<gtk::Image>,
    /// Its text.
    text: glib::WeakRef<gtk::Label>,
}

thread_local! {
    /// The label of this process's file drag, while it lasts.
    static SHOWN: RefCell<Option<ShownLabel>> = const { RefCell::new(None) };
}

/// A new, hidden label for the icon of the drag that starts now; the
/// drop targets fill it in as the drag moves over them.
pub(super) fn drag_label() -> gtk::Box {
    let glyph = icons::image(Icon::ArrowRight, GLYPH_SIZE);
    let text = gtk::Label::builder().single_line_mode(true).build();
    let row = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::Start)
        .css_classes(["drag-label"])
        .visible(false)
        .build();
    row.append(&glyph);
    row.append(&text);
    SHOWN.with(|shown| {
        shown.replace(Some(ShownLabel {
            row: row.downgrade(),
            glyph: glyph.downgrade(),
            text: text.downgrade(),
        }))
    });
    row
}

/// Says on the drag icon what a release would do, or hides the label with
/// `None`. Nothing happens while this process makes no file drag.
pub(crate) fn show_drag_label(label: Option<&DragLabel>) {
    SHOWN.with(|shown| {
        let shown = shown.borrow();
        let Some(shown) = shown.as_ref() else {
            return;
        };
        let (Some(row), Some(glyph), Some(text)) =
            (shown.row.upgrade(), shown.glyph.upgrade(), shown.text.upgrade())
        else {
            return;
        };
        if let Some(label) = label {
            icons::set_icon(&glyph, label.glyph(), GLYPH_SIZE);
            text.set_label(&label.text());
        }
        row.set_visible(label.is_some());
    });
}

/// Forgets the label of a drag that ended.
pub(super) fn forget_drag_label() {
    SHOWN.with(|shown| shown.replace(None));
}

/// The label's text while it shows, for tests.
#[cfg(test)]
pub(crate) fn shown_drag_label() -> Option<String> {
    SHOWN.with(|shown| {
        let shown = shown.borrow();
        let shown = shown.as_ref()?;
        let row = shown.row.upgrade()?;
        let text = shown.text.upgrade()?;
        row.is_visible().then(|| text.label().to_string())
    })
}

/// The label's glyph while it shows, for tests.
#[cfg(test)]
pub(crate) fn shown_drag_glyph() -> Option<String> {
    SHOWN.with(|shown| {
        let shown = shown.borrow();
        let shown = shown.as_ref()?;
        let row = shown.row.upgrade()?;
        let glyph = shown.glyph.upgrade()?;
        row.is_visible()
            .then(|| glyph.icon_name())
            .flatten()
            .map(|name| name.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// parity: DND-028
    #[test]
    fn the_label_names_the_operation_and_the_place_in_windows_words() {
        let label = |verb| DragLabel {
            verb,
            place: "Documents".to_owned(),
        };
        assert_eq!(label(DragVerb::Move).text(), "Move to Documents");
        assert_eq!(label(DragVerb::Copy).text(), "Copy to Documents");
        assert_eq!(label(DragVerb::Link).text(), "Create link in Documents");
        assert_eq!(label(DragVerb::Move).glyph(), Icon::ArrowRight);
        assert_eq!(label(DragVerb::Copy).glyph(), Icon::Add);
        assert_eq!(label(DragVerb::Link).glyph(), Icon::Link);
    }
}
