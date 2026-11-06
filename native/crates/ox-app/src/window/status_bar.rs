// SPDX-License-Identifier: AGPL-3.0-only
//! The status bar at the bottom of the window.
//!
//! Ports `footer.statusbar` in `v2.0.0:desktop/ui/index.html` and `updateStatus`
//! in `v2.0.0:desktop/ui/app.js`, laid out as Windows 11 File Explorer's
//! status bar: the item count ("6 items"), a thin divider and the
//! selection ("1 item selected  19.0 KB", the size only when every
//! selected item is a file), the type-to-select hint, then at the right
//! "Check for updates" while a check found a newer release or a restart
//! to finish one (the build it names is hidden, as Windows 11 Explorer's
//! status bar has none), and the Details and Large icons view buttons, the
//! current view's button highlighted. "Check for updates" takes the accent
//! colour when a check in any window found a newer release (UPD-001). A
//! folder that cannot be watched for changes says so (VIEW-056).
//!
//! Explorer's status bar has none of Dolphin's extras, so neither has
//! this one: no total size of the folder (VIEW-051), no name and type of
//! the selected or hovered item (VIEW-052), no free space (VIEW-053; This
//! PC and Properties show it) and no zoom slider (VIEW-012).
//!
//! [`StatusBar`] is a `GtkBox` subclass laid out by the template
//! `resources/ui/status-bar.ui`; this module adds the glyphs, the build
//! text and the buttons' actions.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::format;
use ox_core::i18n::{gettext, ngettext};

use crate::announcement::announce;
use crate::config::BUILD_NAME;
use crate::folder_view::grid::IconSize;
use crate::folder_view::model::SelectionSummary;
use crate::icons::{self, Icon};
use crate::search::SearchCount;
use crate::update::UpdateState;

use super::folder_pane::FolderView;
use super::window_action::WindowAction;

/// The glyph of the status bar's buttons (ui-spec.md I09; the web app's
/// were 15).
const BUTTON_GLYPH: i32 = 16;

/// The class that mutes a hint for typed text no name starts with.
const MISS_CLASS: &str = "miss";

/// The class of "Check for updates" while an update needs the user.
const UPDATE_CLASS: &str = "update-available";

/// The label and tooltip of the updates button (`#check-updates`).
const CHECK_FOR_UPDATES: &str = crate::i18n::message_id("Check for updates");

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
/// many items are shown, as Windows Explorer counts them ("6 items"), and
/// "Loading…" while a slow listing runs.
pub(super) fn count_text(subject: StatusSubject) -> String {
    let (shown, loading) = match subject {
        StatusSubject::Page => return gettext("Ready"),
        StatusSubject::Search(count) => return count.text(),
        StatusSubject::Folder { shown, loading } => (shown, loading),
    };
    let items = ngettext("{count} item", "{count} items", u64::from(shown))
        .replace("{count}", &format::group_digits(u64::from(shown)));
    if loading {
        format!("{items} · {}", gettext("Loading…"))
    } else {
        items
    }
}

/// The selection (`#status-selected`) as Windows Explorer writes it:
/// "1 item selected" or "3 items selected", then, when every selected item
/// is a file, their size as Explorer writes sizes ("3 items selected
/// 1.24 MB").
pub(super) fn selection_text(selected: SelectionSummary) -> String {
    if selected.count == 0 {
        return String::new();
    }
    let count = ngettext(
        "{count} item selected",
        "{count} items selected",
        u64::from(selected.count),
    )
    .replace("{count}", &format::group_digits(u64::from(selected.count)));
    if selected.all_files {
        format!("{count}  {}", format::short_size(selected.bytes))
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
        /// The thin line between the item count and the selection.
        #[template_child]
        pub(super) selection_divider: TemplateChild<gtk::Separator>,
        /// The selection (`#status-selected`).
        #[template_child]
        pub(super) selection: TemplateChild<gtk::Label>,
        /// The type-to-select hint.
        #[template_child]
        pub(super) typeahead_hint: TemplateChild<gtk::Label>,
        /// Says the folder cannot be watched for changes.
        #[template_child]
        pub(super) watch_state: TemplateChild<gtk::Label>,
        /// The build (`#status-mode`).
        #[template_child]
        pub(super) build: TemplateChild<gtk::Label>,
        /// "Check for updates" (`#check-updates`).
        #[template_child]
        pub(super) check_updates_button: TemplateChild<gtk::Button>,
        /// The glyph of [`Self::check_updates_button`].
        #[template_child]
        pub(super) update_glyph: TemplateChild<gtk::Image>,
        /// "Update available" or "Restart to finish updating".
        #[template_child]
        pub(super) update_label: TemplateChild<gtk::Label>,
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
            crate::i18n::translate_template(&*self.obj(), "status-bar.ui");
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
        // Windows 11 Explorer's status bar names no build; Settings ›
        // About does.
        imp.build.set_visible(false);
        self.finish_check_updates_button();
        let large_icons = FolderView::Icons(IconSize::LARGE);
        show_view_on(
            &imp.details_view_button,
            Icon::TextBulletList,
            FolderView::Details,
        );
        show_view_on(&imp.icons_view_button, Icon::Grid, large_icons);
    }

    /// "Check for updates" opens the Software updates dialog.
    fn finish_check_updates_button(&self) {
        let imp = self.imp();
        let check_updates = &*imp.check_updates_button;
        icons::set_icon(&imp.update_glyph, Icon::ArrowDownload, BUTTON_GLYPH);
        check_updates.set_tooltip_text(Some(ox_core::i18n::gettext_static(CHECK_FOR_UPDATES)));
        WindowAction::CheckUpdates.assign_to(check_updates);
        // Shown only while a check found something that needs the user
        // ([`Self::show_update_notice`]); Settings › About checks at any
        // time.
        check_updates.set_visible(false);
    }

    /// Says on "Check for updates" what a check found that needs the
    /// user: "Update available" for a newer release, or "Restart to finish
    /// updating" for an installed one, in the accent colour, with the
    /// tooltip saying why. `state` needing nothing hides the button.
    pub(super) fn show_update_notice(&self, state: &UpdateState) {
        let imp = self.imp();
        let check_updates = &*imp.check_updates_button;
        let (Some(label), Some(notice)) = (state.status_bar_label(), state.status_bar_notice()) else {
            check_updates.set_visible(false);
            check_updates.remove_css_class(UPDATE_CLASS);
            check_updates.set_tooltip_text(Some(ox_core::i18n::gettext_static(CHECK_FOR_UPDATES)));
            return;
        };
        let glyph = if state.needs_restart() {
            Icon::ArrowClockwise
        } else {
            Icon::ArrowDownload
        };
        icons::set_icon(&imp.update_glyph, glyph, BUTTON_GLYPH);
        imp.update_label.set_text(label);
        check_updates.add_css_class(UPDATE_CLASS);
        let title = ox_core::i18n::gettext_static(CHECK_FOR_UPDATES);
        check_updates.set_tooltip_text(Some(&format!("{title}\n{notice}")));
        check_updates.set_visible(true);
    }

    /// The text of "Check for updates", for tests.
    #[cfg(test)]
    pub(super) fn check_updates_label(&self) -> String {
        self.imp().update_label.text().to_string()
    }

    /// Whether "Check for updates" shows, for tests.
    #[cfg(test)]
    pub(super) fn shows_check_updates(&self) -> bool {
        self.imp().check_updates_button.is_visible()
    }

    /// The tooltip of "Check for updates", for tests.
    #[cfg(test)]
    pub(super) fn check_updates_tooltip(&self) -> String {
        let check_updates = &*self.imp().check_updates_button;
        check_updates.tooltip_text().unwrap_or_default().to_string()
    }

    /// Shows the item count and, after the divider, the selection.
    pub(super) fn set_counts(&self, subject: StatusSubject, selected: SelectionSummary) {
        let imp = self.imp();
        imp.count.set_text(&count_text(subject));
        let selection = match subject {
            StatusSubject::Page => String::new(),
            StatusSubject::Folder { .. } | StatusSubject::Search(_) => selection_text(selected),
        };
        imp.selection_divider.set_visible(!selection.is_empty());
        imp.selection.set_visible(!selection.is_empty());
        imp.selection.set_text(&selection);
    }

    /// Says the folder shown is checked for changes only every `interval`
    /// text, such as "1 minute", or hides the note for `None`.
    pub(super) fn show_unwatched(&self, interval: Option<&str>) {
        let label = &*self.imp().watch_state;
        label.set_visible(interval.is_some());
        if let Some(interval) = interval {
            label.set_tooltip_text(Some(&ox_core::i18n::format_message("Changes made elsewhere are not shown as they happen. OpenXplorer lists this folder again every {interval}; press F5 to list it now.", &[("interval", interval)])));
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
        let details = view == FolderView::Details;
        let icons = matches!(view, FolderView::Icons(_));
        for (button, active) in [
            (&imp.details_view_button, details),
            (&imp.icons_view_button, icons),
        ] {
            if active {
                button.add_css_class("active");
            } else {
                button.remove_css_class("active");
            }
        }
    }

    /// The count and selection as shown, for tests.
    #[cfg(test)]
    pub(super) fn texts(&self) -> (String, String) {
        let imp = self.imp();
        (imp.count.text().to_string(), imp.selection.text().to_string())
    }

    /// Whether the divider before the selection shows, for tests.
    #[cfg(test)]
    pub(super) fn shows_selection_divider(&self) -> bool {
        self.imp().selection_divider.is_visible()
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

    /// The count is Windows Explorer's "N items", with no size added.
    ///
    /// parity: VIEW-050
    #[test]
    fn the_status_counts_items_and_says_when_it_is_loading() {
        assert_eq!(folder_count(1, false), "1 item");
        assert_eq!(folder_count(6, false), "6 items");
        assert_eq!(folder_count(4, true), "4 items · Loading…");
    }

    /// The selection reads as Windows Explorer's: "1 item selected  19.0
    /// KB", the size written as Explorer writes it, and no size once a
    /// folder is among the selected items.
    ///
    /// parity: VIEW-050
    #[test]
    fn the_selection_shows_its_count_and_file_size() {
        assert_eq!(selection_text(SelectionSummary::default()), "");
        let one_file = SelectionSummary {
            count: 1,
            bytes: 19_456,
            all_files: true,
        };
        assert_eq!(selection_text(one_file), "1 item selected  19.0 KB");
        let three_files = SelectionSummary {
            count: 3,
            bytes: 1_300_000,
            all_files: true,
        };
        assert_eq!(selection_text(three_files), "3 items selected  1.23 MB");
        let folders = SelectionSummary {
            count: 3,
            bytes: 0,
            all_files: false,
        };
        assert_eq!(selection_text(folders), "3 items selected");
    }

    /// parity: VIEW-050
    #[test]
    fn a_landing_page_is_ready() {
        assert_eq!(count_text(StatusSubject::Page), "Ready");
    }
}
