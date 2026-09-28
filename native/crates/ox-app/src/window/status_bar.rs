// SPDX-License-Identifier: AGPL-3.0-only
//! The status bar at the bottom of the window.
//!
//! Ports `footer.statusbar` in `desktop/ui/index.html` and `updateStatus`
//! in `desktop/ui/app.js`: the item count, the selection, the
//! type-to-select hint, then at the right the build, "Check for updates"
//! and the Details and Large icons view buttons, the current view's
//! button highlighted.

use gtk::pango;
use gtk::prelude::*;
use ox_core::format;

use crate::folder_view::grid::IconSize;
use crate::folder_view::model::SelectionSummary;
use crate::icons::{self, Glyph};

use super::content::FolderView;
use super::unported;
use super::window_action::WindowAction;

/// The glyph of the status bar's buttons (ui-spec.md I09; the web app's
/// were 15).
const BUTTON_GLYPH: i32 = 16;

/// The build shown at the right (`#status-mode`).
const BUILD_TEXT: &str = concat!("OpenXplorer ", env!("CARGO_PKG_VERSION"), " native preview");

/// The class that mutes a hint for typed text no name starts with.
const MISS_CLASS: &str = "miss";

/// What the status bar reports about the active tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum StatusSubject {
    /// A landing page, which has nothing to count.
    Page,
    /// A folder listing.
    Folder {
        /// Items shown after filtering.
        shown: u32,
        /// The folder is still being listed.
        loading: bool,
    },
}

/// Whether typed text found a name, which decides how its hint is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TypeaheadMatch {
    /// A name starts with the typed text: the hint is in the accent colour.
    Found,
    /// No name does: the hint is muted, as it is not an error.
    Missed,
}

/// The item count (`#status-count`): "Ready" on a landing page, else how
/// many items are shown, and "Loading…" while the folder is listed.
pub(super) fn count_text(subject: StatusSubject) -> String {
    let StatusSubject::Folder { shown, loading } = subject else {
        return "Ready".to_owned();
    };
    let items = if shown == 1 {
        "1 item".to_owned()
    } else {
        format!("{shown} items")
    };
    if loading {
        format!("{items} · Loading…")
    } else {
        items
    }
}

/// The selection (`#status-selected`): how many items are selected and,
/// as Windows Explorer shows, how large the selected files are.
pub(super) fn selection_text(selected: SelectionSummary) -> String {
    if selected.count == 0 {
        return String::new();
    }
    let count = format!("{} selected", selected.count);
    if selected.has_files {
        format!("{count}  {}", format::pretty_bytes(selected.bytes))
    } else {
        count
    }
}

/// The status bar's widgets.
#[derive(Debug)]
pub(super) struct StatusBar {
    /// The bar.
    pub root: gtk::Box,
    count: gtk::Label,
    selection: gtk::Label,
    typeahead_hint: gtk::Label,
    build: gtk::Label,
    details_view: gtk::Button,
    icons_view: gtk::Button,
}

impl StatusBar {
    /// An empty status bar.
    pub fn new() -> Self {
        let count = gtk::Label::builder().label("Ready").xalign(0.0).build();
        let selection = gtk::Label::builder().xalign(0.0).build();
        let typeahead_hint = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(pango::EllipsizeMode::End)
            .css_classes(["typeahead-hint"])
            .build();
        let spacer = gtk::Box::builder().hexpand(true).build();
        let build = gtk::Label::builder()
            .label(BUILD_TEXT)
            .ellipsize(pango::EllipsizeMode::End)
            .css_classes(["status-mode"])
            .build();
        let details_view = view_button(Glyph::List, "Details view", FolderView::Details);
        let icons_view = view_button(Glyph::Grid, "Large icons", FolderView::Icons(IconSize::Large));
        let root = gtk::Box::builder().spacing(18).css_classes(["statusbar"]).build();
        root.append(&count);
        root.append(&selection);
        root.append(&typeahead_hint);
        root.append(&spacer);
        root.append(&build);
        root.append(&check_updates_button());
        root.append(&details_view);
        root.append(&icons_view);
        Self {
            root,
            count,
            selection,
            typeahead_hint,
            build,
            details_view,
            icons_view,
        }
    }

    /// Shows or hides the build text, which a compact window has no room
    /// for (`.status-mode{display:none}` at 680 pixels).
    pub fn set_build_visible(&self, visible: bool) {
        self.build.set_visible(visible);
    }

    /// Shows the item count and the selection.
    pub fn show(&self, subject: StatusSubject, selected: SelectionSummary) {
        self.count.set_text(&count_text(subject));
        let selection = match subject {
            StatusSubject::Page => String::new(),
            StatusSubject::Folder { .. } => selection_text(selected),
        };
        self.selection.set_visible(!selection.is_empty());
        self.selection.set_text(&selection);
    }

    /// Shows the type-to-select `hint`, drawn as its `outcome` asks.
    pub fn show_typeahead_hint(&self, hint: &str, outcome: TypeaheadMatch) {
        self.typeahead_hint.set_text(hint);
        match outcome {
            TypeaheadMatch::Found => self.typeahead_hint.remove_css_class(MISS_CLASS),
            TypeaheadMatch::Missed => self.typeahead_hint.add_css_class(MISS_CLASS),
        }
    }

    /// Clears the type-to-select hint.
    pub fn clear_typeahead_hint(&self) {
        self.typeahead_hint.set_text("");
    }

    /// Highlights the button of `view`. Every icon size counts as the icon
    /// view, as the Python app's single grid view did.
    pub fn show_view(&self, view: FolderView) {
        let (on, off) = match view {
            FolderView::Details => (&self.details_view, &self.icons_view),
            FolderView::Icons(_) => (&self.icons_view, &self.details_view),
        };
        on.add_css_class("active");
        off.remove_css_class("active");
    }

    /// The count and selection as shown, for tests.
    #[cfg(test)]
    pub fn texts(&self) -> (String, String) {
        (self.count.text().to_string(), self.selection.text().to_string())
    }

    /// The type-to-select hint's label, for tests.
    #[cfg(test)]
    pub fn typeahead_hint_label(&self) -> &gtk::Label {
        &self.typeahead_hint
    }

    /// The view buttons that show as active, for tests.
    #[cfg(test)]
    pub fn active_view_buttons(&self) -> Vec<String> {
        [&self.details_view, &self.icons_view]
            .into_iter()
            .filter(|button| button.has_css_class("active"))
            .filter_map(|button| button.tooltip_text().map(|text| text.to_string()))
            .collect()
    }
}

fn view_button(glyph: Glyph, tooltip: &str, view: FolderView) -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::glyph(glyph, BUTTON_GLYPH))
        .tooltip_text(tooltip)
        .action_name(WindowAction::View.detailed_name())
        .action_target(&view.key().to_variant())
        .valign(gtk::Align::Center)
        .build();
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    button
}

/// "Check for updates" (`#check-updates`), disabled until the update flow
/// is ported.
fn check_updates_button() -> gtk::Button {
    let button = gtk::Button::builder()
        .child(&icons::glyph(Glyph::Refresh, BUTTON_GLYPH))
        .tooltip_text(unported::tooltip(WindowAction::CheckUpdates, "Check for updates"))
        .action_name(WindowAction::CheckUpdates.detailed_name())
        .valign(gtk::Align::Center)
        .build();
    button.update_property(&[gtk::accessible::Property::Label("Check for updates")]);
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_count(shown: u32, loading: bool) -> String {
        count_text(StatusSubject::Folder { shown, loading })
    }

    #[test]
    fn the_status_counts_items_and_says_when_it_is_loading() {
        assert_eq!(folder_count(1, false), "1 item");
        assert_eq!(folder_count(4, true), "4 items · Loading…");
    }

    #[test]
    fn the_selection_shows_its_count_and_file_size() {
        assert_eq!(selection_text(SelectionSummary::default()), "");
        let two_files = SelectionSummary {
            count: 2,
            bytes: 2048,
            has_files: true,
        };
        let size = format::pretty_bytes(2048);
        assert_eq!(selection_text(two_files), format!("2 selected  {size}"));
        let folders = SelectionSummary {
            count: 3,
            bytes: 0,
            has_files: false,
        };
        assert_eq!(selection_text(folders), "3 selected");
    }

    #[test]
    fn a_landing_page_is_ready() {
        assert_eq!(count_text(StatusSubject::Page), "Ready");
    }
}
