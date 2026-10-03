// SPDX-License-Identifier: AGPL-3.0-only
//! The status bar at the bottom of the window.
//!
//! Ports `footer.statusbar` in `v2.0.0:desktop/ui/index.html` and `updateStatus`
//! in `v2.0.0:desktop/ui/app.js`: the item count, the selection, the
//! type-to-select hint, then at the right the volume's free space, the
//! build, "Check for updates"
//! and the Details and Large icons view buttons, the current view's
//! button highlighted. In the icon view a slider beside them zooms the
//! icons, as Dolphin's status bar does (VIEW-010). "Check for updates" takes the accent colour when a
//! check in any window found a newer release (UPD-001).
//!
//! As Dolphin's status bar does, the count adds the size of the files
//! listed (VIEW-051), one selected item is described by name, type and
//! size, and so is the item under the pointer until it leaves (VIEW-052);
//! the free space has a bar of how full the volume is (VIEW-053), and a
//! folder that cannot be watched for changes says so (VIEW-056).
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
use crate::folder_view::details::column_text::item_count_text;
use crate::folder_view::grid::IconSize;
use crate::folder_view::item::FileItem;
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
        /// The total size of the files shown.
        bytes: u64,
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
/// many items are shown and how large the files among them are, and
/// "Loading…" while a slow listing runs.
pub(super) fn count_text(subject: StatusSubject) -> String {
    let (shown, bytes, loading) = match subject {
        StatusSubject::Page => return gettext("Ready"),
        StatusSubject::Search(count) => return count.text(),
        StatusSubject::Folder {
            shown,
            bytes,
            loading,
        } => (shown, bytes, loading),
    };
    let mut items =
        ngettext("{count} item", "{count} items", u64::from(shown)).replace("{count}", &shown.to_string());
    if bytes > 0 {
        items = format!("{items}  {}", format::pretty_bytes(bytes));
    }
    if loading {
        format!("{items} · {}", gettext("Loading…"))
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
    let count = ngettext("{count} selected", "{count} selected", u64::from(selected.count))
        .replace("{count}", &selected.count.to_string());
    if selected.has_files {
        format!("{count}  {}", format::pretty_bytes(selected.bytes))
    } else {
        count
    }
}

/// What the status bar says about one item, the one selected or the one
/// under the pointer: its name, its type and its size or item count
/// (`KFileItem::getStatusBarInfo` in Dolphin).
pub(super) fn item_text(item: &FileItem) -> String {
    let entry = item.entry();
    let size = match (item.folder_size(), item.item_count()) {
        (Some(measured), _) => Some(measured.size_text()),
        (None, Some(count)) => Some(item_count_text(count)),
        (None, None) => item.file_size().map(format::pretty_bytes),
    };
    let mut parts = vec![entry.name.clone(), entry.type_label.clone()];
    parts.extend(size);
    parts.retain(|part| !part.is_empty());
    parts.join(" · ")
}

mod imp {
    use std::cell::{Cell, RefCell};

    use gtk::glib;
    use gtk::subclass::prelude::*;

    use crate::folder_view::grid::IconSize;

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
        /// Says the folder cannot be watched for changes.
        #[template_child]
        pub(super) watch_state: TemplateChild<gtk::Label>,
        /// The free space and the capacity bar.
        #[template_child]
        pub(super) free_space_box: TemplateChild<gtk::Box>,
        /// The current volume's free space.
        #[template_child]
        pub(super) free_space: TemplateChild<gtk::Label>,
        /// How full the current volume is.
        #[template_child]
        pub(super) capacity: TemplateChild<gtk::ProgressBar>,
        /// The build (`#status-mode`).
        #[template_child]
        pub(super) build: TemplateChild<gtk::Label>,
        /// "Check for updates" (`#check-updates`).
        #[template_child]
        pub(super) check_updates_button: TemplateChild<gtk::Button>,
        /// Zooms the icon view through its icon sizes.
        #[template_child]
        pub(super) zoom_slider: TemplateChild<gtk::Scale>,
        /// Shows the details view.
        #[template_child]
        pub(super) details_view_button: TemplateChild<gtk::Button>,
        /// Shows the Large icons view.
        #[template_child]
        pub(super) icons_view_button: TemplateChild<gtk::Button>,
        /// The selection's text.
        pub(super) selection_text: RefCell<String>,
        /// The item under the pointer, described, while there is one.
        pub(super) hovered_text: RefCell<Option<String>>,
        /// The icon size the window shows, which the slider then shows.
        pub(super) shown_level: Cell<Option<IconSize>>,
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
        self.finish_check_updates_button();
        let large_icons = FolderView::Icons(IconSize::LARGE);
        show_view_on(
            &imp.details_view_button,
            Icon::TextBulletList,
            FolderView::Details,
        );
        show_view_on(&imp.icons_view_button, Icon::Grid, large_icons);
        self.finish_zoom_slider();
    }

    /// The slider steps through every icon size and shows the icon view
    /// at the one it is moved to.
    fn finish_zoom_slider(&self) {
        let slider = &*self.imp().zoom_slider;
        slider.set_range(0.0, level_value(IconSize::LARGEST));
        slider.set_increments(1.0, 1.0);
        slider.set_round_digits(0);
        slider.connect_value_changed(glib::clone!(
            #[weak(rename_to = bar)]
            self,
            move |slider| {
                let level = IconSize::at_index(zoom_level_at(slider.value()));
                if bar.imp().shown_level.get() == Some(level) {
                    return;
                }
                let view = FolderView::Icons(level);
                WindowAction::View.activate_from(slider, Some(&view.as_str().to_variant()));
            }
        ));
    }

    /// "Check for updates" opens the Software updates dialog.
    fn finish_check_updates_button(&self) {
        let check_updates = &*self.imp().check_updates_button;
        check_updates.set_child(Some(&icons::image(Icon::ArrowClockwise, BUTTON_GLYPH)));
        check_updates.set_tooltip_text(Some(ox_core::i18n::gettext_static(CHECK_FOR_UPDATES)));
        WindowAction::CheckUpdates.assign_to(check_updates);
    }

    /// Says on "Check for updates" what a check found that needs the
    /// user: a newer release, or a restart into an installed one. The
    /// button then takes the accent colour, and its tooltip says why.
    pub(super) fn show_update_notice(&self, notice: Option<&str>) {
        let check_updates = &*self.imp().check_updates_button;
        let Some(notice) = notice else {
            check_updates.remove_css_class(UPDATE_CLASS);
            check_updates.set_tooltip_text(Some(ox_core::i18n::gettext_static(CHECK_FOR_UPDATES)));
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

    /// Shows the item count and the selection; one selected item is
    /// described by `single`, its [`item_text`].
    pub(super) fn set_counts(
        &self,
        subject: StatusSubject,
        selected: SelectionSummary,
        single: Option<String>,
    ) {
        let imp = self.imp();
        imp.count.set_text(&count_text(subject));
        let selection = match (subject, single) {
            (StatusSubject::Page, _) => String::new(),
            (_, Some(single)) if selected.count == 1 => single,
            (StatusSubject::Folder { .. } | StatusSubject::Search(_), _) => selection_text(selected),
        };
        imp.selection_text.replace(selection);
        self.show_selection_text();
    }

    /// Describes `hovered`, the item under the pointer, in place of the
    /// selection, or the selection again for `None`.
    pub(super) fn show_hovered(&self, hovered: Option<String>) {
        if *self.imp().hovered_text.borrow() != hovered {
            self.imp().hovered_text.replace(hovered);
            self.show_selection_text();
        }
    }

    /// Shows the item under the pointer, else the selection.
    fn show_selection_text(&self) {
        let imp = self.imp();
        let text = imp
            .hovered_text
            .borrow()
            .clone()
            .unwrap_or_else(|| imp.selection_text.borrow().clone());
        imp.selection.set_visible(!text.is_empty());
        imp.selection.set_text(&text);
    }

    /// Shows how much room `capacity` has left, or hides the free space.
    pub(super) fn show_free_space(&self, capacity: Option<Capacity>) {
        let imp = self.imp();
        imp.free_space_box.set_visible(capacity.is_some());
        let Some(capacity) = capacity else { return };
        let tooltip = capacity.free_tooltip();
        imp.free_space.set_text(&capacity.free_text());
        imp.free_space_box.set_tooltip_text(Some(&tooltip));
        imp.capacity.set_fraction(capacity.used_share());
        if capacity.is_nearly_full() {
            imp.capacity.add_css_class("full");
        } else {
            imp.capacity.remove_css_class("full");
        }
        imp.capacity
            .update_property(&[gtk::accessible::Property::Label(&tooltip)]);
    }

    /// The free space and its bar, which a click on offers the disk-usage
    /// tools.
    pub(super) fn free_space_widget(&self) -> gtk::Widget {
        self.imp().free_space_box.get().upcast()
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
        let slider = &*imp.zoom_slider;
        slider.set_visible(icons);
        if let FolderView::Icons(size) = view {
            // Recorded first, so moving the slider here runs no action.
            imp.shown_level.set(Some(size));
            slider.set_value(level_value(size));
        }
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

    /// The icon-size slider, for tests.
    #[cfg(test)]
    pub(super) fn zoom_slider(&self) -> gtk::Scale {
        self.imp().zoom_slider.get()
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

/// The slider position of `size`.
fn level_value(size: IconSize) -> f64 {
    // A handful of levels: the index converts exactly.
    f64::from(u8::try_from(size.index()).unwrap_or(u8::MAX))
}

/// The level nearest the slider position `value`.
fn zoom_level_at(value: f64) -> usize {
    // The slider's range is 0 to the last level, so the cast is exact.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a small, non-negative index"
    )]
    let level = value.round().max(0.0) as usize;
    level
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
        count_text(StatusSubject::Folder {
            shown,
            bytes: 0,
            loading,
        })
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

    /// The folder's count adds the size of its files, and one item is
    /// described by name, type and size.
    ///
    /// parity: VIEW-051, VIEW-052
    #[test]
    fn the_count_adds_the_files_size_and_one_item_is_described() {
        let folder = StatusSubject::Folder {
            shown: 3,
            bytes: 2048,
            loading: false,
        };
        assert_eq!(
            count_text(folder),
            format!("3 items  {}", format::pretty_bytes(2048))
        );
        let mut entry = crate::test_support::file_entry("Report.pdf");
        entry.size = Some(2048);
        entry.type_label = "PDF document".to_owned();
        let report = FileItem::new(entry);
        assert_eq!(
            item_text(&report),
            format!("Report.pdf · PDF document · {}", format::pretty_bytes(2048))
        );
    }

    /// parity: VIEW-050
    #[test]
    fn a_landing_page_is_ready() {
        assert_eq!(count_text(StatusSubject::Page), "Ready");
    }

    /// A hovered filename must not force a narrow window wider, including
    /// when font metrics grow at a different display scale.
    #[gtk::test]
    fn long_hover_descriptions_keep_the_status_bars_minimum_width() {
        use crate::test_support::harness::{Fixture, TestWindow};

        let fixture = Fixture::standard();
        let test = TestWindow::open(&fixture.uri());
        let bar = test.window.status_bar();
        bar.show_hovered(Some("Notes.txt".to_owned()));
        let minimum = bar.measure(gtk::Orientation::Horizontal, -1).0;
        let description = format!("{}.txt · Text document · 2 KiB", "Project notes ".repeat(20));

        bar.show_hovered(Some(description.clone()));

        assert_eq!(bar.measure(gtk::Orientation::Horizontal, -1).0, minimum);
        assert_eq!(bar.texts().1, description, "the full text remains accessible");
    }
}
