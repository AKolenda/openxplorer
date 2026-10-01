// SPDX-License-Identifier: AGPL-3.0-only
//! The status bar at the bottom of the window.
//!
//! Ports `footer.statusbar` in `v2.0.0:desktop/ui/index.html` and `updateStatus`
//! in `v2.0.0:desktop/ui/app.js`: the item count, the selection, the
//! type-to-select hint, then at the right the volume's free space, the
//! build, "Check for updates"
//! and the Details and Large icons view buttons, the current view's
//! button highlighted. "Check for updates" takes the accent colour when a
//! check in any window found a newer release (UPD-001).
//!
//! [`StatusBar`] is a `GtkBox` subclass laid out by the template
//! `resources/ui/status-bar.ui`; this module adds the glyphs, the build
//! text and the buttons' actions.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::format;

use crate::announcement::announce;
use crate::config::BUILD_NAME;
use crate::folder_view::grid::IconSize;
use crate::folder_view::model::SelectionSummary;
use crate::icons::{self, Icon};
use crate::search::SearchCount;

use super::folder_pane::FolderView;
use super::landing::Capacity;
use super::window_action::WindowAction;

/// The glyph of the status bar's buttons (ui-spec.md I09; the web app's
/// were 15).
const BUTTON_GLYPH: i32 = 16;

/// The class that mutes a hint for typed text no name starts with.
const MISS_CLASS: &str = "miss";

/// The class of "Check for updates" while an update needs the user.
const UPDATE_CLASS: &str = "update-available";

/// The label and tooltip of the updates button (`#check-updates`).
const CHECK_FOR_UPDATES: &str = "Check for updates";

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
    /// The results of a search (VIEW-050).
    Search(SearchCount),
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
/// many items are shown, and "Loading…" while a slow listing runs.
pub(super) fn count_text(subject: StatusSubject) -> String {
    let (shown, loading) = match subject {
        StatusSubject::Page => return "Ready".to_owned(),
        StatusSubject::Search(count) => return count.text(),
        StatusSubject::Folder { shown, loading } => (shown, loading),
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

mod imp {
    use gtk::glib;
    use gtk::subclass::prelude::*;

    /// Private state of [`super::StatusBar`]: the template's widgets.
    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(file = "../../resources/ui/status-bar.ui")]
    pub(crate) struct StatusBar {
        /// The item count (`#status-count`).
        #[template_child]
        pub(super) count: TemplateChild<gtk::Label>,
        /// The selection (`#status-selected`).
        #[template_child]
        pub(super) selection: TemplateChild<gtk::Label>,
        /// The type-to-select hint.
        #[template_child]
        pub(super) typeahead_hint: TemplateChild<gtk::Label>,
        /// The current volume's free space.
        #[template_child]
        pub(super) free_space: TemplateChild<gtk::Label>,
        /// The build (`#status-mode`).
        #[template_child]
        pub(super) build: TemplateChild<gtk::Label>,
        /// "Check for updates" (`#check-updates`).
        #[template_child]
        pub(super) check_updates_button: TemplateChild<gtk::Button>,
        /// Shows the details view.
        #[template_child]
        pub(super) details_view_button: TemplateChild<gtk::Button>,
        /// Shows the Large icons view.
        #[template_child]
        pub(super) icons_view_button: TemplateChild<gtk::Button>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for StatusBar {
        const NAME: &'static str = "OxStatusBar";
        type Type = super::StatusBar;
        type ParentType = gtk::Box;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(bar: &glib::subclass::InitializingObject<Self>) {
            bar.init_template();
        }
    }

    impl ObjectImpl for StatusBar {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().finish_template();
        }
    }

    impl WidgetImpl for StatusBar {}
    impl BoxImpl for StatusBar {}
}

glib::wrapper! {
    /// The status bar at the bottom of the window.
    pub(crate) struct StatusBar(ObjectSubclass<imp::StatusBar>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl StatusBar {
    /// Adds what the template cannot express: the build, the buttons'
    /// glyphs and their actions.
    fn finish_template(&self) {
        let imp = self.imp();
        imp.build.set_text(BUILD_NAME);
        self.finish_check_updates_button();
        let large_icons = FolderView::Icons(IconSize::Large);
        show_view_on(
            &imp.details_view_button,
            Icon::TextBulletList,
            FolderView::Details,
        );
        show_view_on(&imp.icons_view_button, Icon::Grid, large_icons);
    }

    /// "Check for updates" opens the Software updates dialog.
    fn finish_check_updates_button(&self) {
        let check_updates = &*self.imp().check_updates_button;
        check_updates.set_child(Some(&icons::image(Icon::ArrowClockwise, BUTTON_GLYPH)));
        check_updates.set_tooltip_text(Some(CHECK_FOR_UPDATES));
        WindowAction::CheckUpdates.assign_to(check_updates);
    }

    /// Says on "Check for updates" what a check found that needs the
    /// user: a newer release, or a restart into an installed one. The
    /// button then takes the accent colour, and its tooltip says why.
    pub(super) fn show_update_notice(&self, notice: Option<&str>) {
        let check_updates = &*self.imp().check_updates_button;
        let Some(notice) = notice else {
            check_updates.remove_css_class(UPDATE_CLASS);
            check_updates.set_tooltip_text(Some(CHECK_FOR_UPDATES));
            return;
        };
        check_updates.add_css_class(UPDATE_CLASS);
        check_updates.set_tooltip_text(Some(&format!("{CHECK_FOR_UPDATES}\n{notice}")));
    }

    /// The tooltip of "Check for updates", for tests.
    #[cfg(test)]
    pub(super) fn check_updates_tooltip(&self) -> String {
        let check_updates = &*self.imp().check_updates_button;
        check_updates.tooltip_text().unwrap_or_default().to_string()
    }

    /// Shows or hides the build text, which a compact window has no room
    /// for (`.status-mode{display:none}` at 680 pixels).
    pub(super) fn set_build_visible(&self, visible: bool) {
        self.imp().build.set_visible(visible);
    }

    /// Shows the item count and the selection.
    pub(super) fn set_counts(&self, subject: StatusSubject, selected: SelectionSummary) {
        let imp = self.imp();
        imp.count.set_text(&count_text(subject));
        let selection = match subject {
            StatusSubject::Page => String::new(),
            StatusSubject::Folder { .. } | StatusSubject::Search(_) => selection_text(selected),
        };
        imp.selection.set_visible(!selection.is_empty());
        imp.selection.set_text(&selection);
    }

    /// Shows how much room `capacity` has left, or hides the free space.
    pub(super) fn show_free_space(&self, capacity: Option<Capacity>) {
        let label = &*self.imp().free_space;
        label.set_visible(capacity.is_some());
        if let Some(capacity) = capacity {
            label.set_text(&capacity.free_text());
            label.set_tooltip_text(Some(&capacity.free_tooltip()));
        }
    }

    /// Shows the type-to-select `hint`, drawn as its `outcome` asks, and
    /// reads it to screen readers without moving focus, as the polite
    /// live region `#type-select-status` in index.html does.
    pub(super) fn show_typeahead_hint(&self, hint: &str, outcome: TypeaheadMatch) {
        let label = &*self.imp().typeahead_hint;
        label.set_text(hint);
        announce(label, hint, gtk::AccessibleAnnouncementPriority::Medium);
        match outcome {
            TypeaheadMatch::Found => label.remove_css_class(MISS_CLASS),
            TypeaheadMatch::Missed => label.add_css_class(MISS_CLASS),
        }
    }

    /// Clears the type-to-select hint.
    pub(super) fn clear_typeahead_hint(&self) {
        self.imp().typeahead_hint.set_text("");
    }

    /// Highlights the button of `view`. Every icon size counts as the icon
    /// view, as the Python app's single grid view did.
    pub(super) fn show_view(&self, view: FolderView) {
        let imp = self.imp();
        let (on, off) = match view {
            FolderView::Details => (&imp.details_view_button, &imp.icons_view_button),
            FolderView::Icons(_) => (&imp.icons_view_button, &imp.details_view_button),
        };
        on.add_css_class("active");
        off.remove_css_class("active");
    }

    /// The count and selection as shown, for tests.
    #[cfg(test)]
    pub(super) fn texts(&self) -> (String, String) {
        let imp = self.imp();
        (imp.count.text().to_string(), imp.selection.text().to_string())
    }

    /// The type-to-select hint's label, for tests.
    #[cfg(test)]
    pub(super) fn typeahead_hint_label(&self) -> gtk::Label {
        self.imp().typeahead_hint.get()
    }

    /// The view buttons that show as active, for tests.
    #[cfg(test)]
    pub(super) fn active_view_buttons(&self) -> Vec<String> {
        let imp = self.imp();
        [&imp.details_view_button, &imp.icons_view_button]
            .into_iter()
            .filter(|button| button.has_css_class("active"))
            .filter_map(|button| button.tooltip_text().map(|text| text.to_string()))
            .collect()
    }
}

/// Makes `button` show `glyph` and switch to `view`.
fn show_view_on(button: &gtk::Button, glyph: Icon, view: FolderView) {
    button.set_child(Some(&icons::image(glyph, BUTTON_GLYPH)));
    WindowAction::View.assign_with_target_to(button, &view.as_str().to_variant());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder_count(shown: u32, loading: bool) -> String {
        count_text(StatusSubject::Folder { shown, loading })
    }

    /// parity: VIEW-050
    #[test]
    fn the_status_counts_items_and_says_when_it_is_loading() {
        assert_eq!(folder_count(1, false), "1 item");
        assert_eq!(folder_count(4, true), "4 items · Loading…");
    }

    /// parity: VIEW-050
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

    /// parity: VIEW-050
    #[test]
    fn a_landing_page_is_ready() {
        assert_eq!(count_text(StatusSubject::Page), "Ready");
    }
}
