// SPDX-License-Identifier: AGPL-3.0-only
//! The search box at the right of the navigation row.
//!
//! Ports `.search-wrap` in `desktop/ui/index.html` and `style.css`: a
//! bordered field reading "Search <folder>" with a thin magnifier at its
//! right end, as in Windows 11. A `GtkSearchEntry` does the typing, so its
//! delayed `search-changed`, Escape handling and clear icon stay; only its
//! own magnifier, which GTK always draws first, is hidden in favour of the
//! trailing one.

use gtk::prelude::*;

use crate::icons::{self, Glyph};

/// How long typing pauses before the folder is filtered (`queueSearch`).
const SEARCH_DELAY_MS: u32 = 120;

/// The search box's widgets.
#[derive(Debug)]
pub(super) struct SearchBox {
    /// The bordered box in the navigation row.
    pub root: gtk::Box,
    /// The field the user types in.
    pub entry: gtk::SearchEntry,
}

impl SearchBox {
    /// An empty search box.
    pub fn new() -> Self {
        let entry = gtk::SearchEntry::builder()
            .hexpand(true)
            .search_delay(SEARCH_DELAY_MS)
            .tooltip_text("Search file names in this folder")
            .build();
        entry.update_property(&[gtk::accessible::Property::Label("Search filenames and paths")]);
        // The box has a fixed width (`.search-wrap{width:235px}`), so the
        // entry asks for no more than one character.
        entry.set_width_chars(1);
        entry.set_max_width_chars(1);
        hide_leading_magnifier(&entry);
        let magnifier = icons::glyph(Glyph::Search, 15);
        magnifier.add_css_class("search-icon");
        // Not expanding, although the entry inside does: the address bar
        // takes the rest of the row.
        let root = gtk::Box::builder()
            .valign(gtk::Align::Center)
            .hexpand(false)
            .css_classes(["search-wrap"])
            .build();
        root.append(&entry);
        root.append(&magnifier);
        Self { root, entry }
    }

    /// Names the folder the box searches: "Search Documents".
    pub fn set_folder_title(&self, title: &str) {
        self.entry.set_placeholder_text(Some(&format!("Search {title}")));
    }

    /// Enables the box in folders and disables it on landing pages.
    pub fn set_enabled(&self, enabled: bool) {
        self.root.set_sensitive(enabled);
    }

    /// Empties the box, which ends the filter.
    pub fn clear(&self) {
        self.entry.set_text("");
    }
}

/// Hides the magnifier `GtkSearchEntry` puts before the text; it is the
/// entry's first child image in GTK 4.
fn hide_leading_magnifier(entry: &gtk::SearchEntry) {
    let leading = entry.first_child().and_downcast::<gtk::Image>();
    if let Some(image) = leading {
        image.set_visible(false);
    }
}
