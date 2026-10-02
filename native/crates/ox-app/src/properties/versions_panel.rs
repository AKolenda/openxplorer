// SPDX-License-Identifier: AGPL-3.0-only
//! The Previous versions tab of the Properties dialog (PROP-019).
//!
//! Ports `renderVersionsPanel` in `v2.0.0:desktop/ui/app.js`: what the tab can and
//! cannot find, Refresh and Snapshot source…, "Checking readable snapshot
//! folders…" while the lookup runs, then the versions under their column
//! titles, or "No accessible previous versions" with the reason, and the
//! notes on truncation, warnings and the supported layouts. Refreshing,
//! opening the Snapshot source form or closing the dialog cancels a
//! lookup in progress, and a stale lookup's result is ignored.

use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{ItemKind, LocationContext};
use ox_core::transfer::Cancellation;
use ox_core::versions::{PreviousVersions, VersionList, VersionsError};

use super::snapshot_source::{fill_source_form, SourceItem};
use super::version_row::version_row;
use super::PropertiesTarget;
use crate::dialog::{note, quiet_text};
use crate::icons::{self, Icon};
use crate::window::ButtonStyle;

/// What the tab browses, and what it does not.
const VERSIONS_INTRO: &str = "Browse real snapshot or backup folders exposed by your server. This build \
                              does not enumerate Windows SMB shadow-copy protocol responses or create \
                              snapshots.";
/// Shown while the lookup runs.
const CHECKING: &str = "Checking readable snapshot folders…";
/// Above the list: where dates come from.
const DATE_NOTE: &str = "Dates are read from snapshot names. Times stay as written; UTC is marked when \
                         supplied.";
/// The heading of an empty result.
const NO_VERSIONS: &str = "No accessible previous versions";
/// Under a truncated list.
const TRUNCATED_NOTE: &str =
    "Showing at most 100 versions. Configure a narrower snapshot folder to see more.";
/// At the end of the tab: what counts as a previous version.
const LAYOUTS_NOTE: &str = "Snapshots must already exist and be readable through your NAS. Built-in \
                            layouts include .snapshot, #snapshot, .zfs/snapshot, and Snapper \
                            .snapshots/<id>/snapshot. Restore a copy never overwrites the live item. A \
                            cached search result is not a previous version.";
/// The empty result's clock glyph (`icon('clock', 30)`).
const EMPTY_GLYPH: i32 = 30;
/// The width of a row's date cell (`.version-date`), which its title
/// shares.
const DATE_COLUMN_WIDTH: i32 = 130;
/// The width of the Actions title: a row's Browse and Restore buttons.
const ACTIONS_COLUMN_WIDTH: i32 = 210;
/// The glyph size of the toolbar's buttons.
const TOOLBAR_GLYPH: i32 = 14;

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::sync::Arc;

    use gtk::glib;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use ox_core::location::{ItemKind, LocationContext};
    use ox_core::transfer::Cancellation;
    use ox_core::versions::PreviousVersions;

    /// Private state of [`super::VersionsPanel`].
    #[derive(Debug, Default)]
    pub(crate) struct VersionsPanel {
        /// The item whose versions are listed.
        pub(super) uri: RefCell<String>,
        /// Whether the item is a file or a folder.
        pub(super) kind: Cell<Option<ItemKind>>,
        /// The shared previous-versions service; set by `new`.
        pub(super) versions: OnceCell<Arc<PreviousVersions>>,
        /// Display names for collections.
        pub(super) locations: RefCell<LocationContext>,
        /// The lookup in progress, to cancel.
        pub(super) lookup: RefCell<Option<Cancellation>>,
        /// Counts lookups, so the result of an older one is dropped.
        pub(super) generation: Cell<u64>,
        /// Set once the first lookup started.
        pub(super) is_loaded: Cell<bool>,
        /// The list of versions, or the message in its place.
        pub(super) list: RefCell<Option<gtk::Box>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VersionsPanel {
        const NAME: &'static str = "OxVersionsPanel";
        type Type = super::VersionsPanel;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for VersionsPanel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().set_orientation(gtk::Orientation::Vertical);
        }
    }

    impl WidgetImpl for VersionsPanel {}
    impl BoxImpl for VersionsPanel {}
}

glib::wrapper! {
    /// The Previous versions tab.
    pub(crate) struct VersionsPanel(ObjectSubclass<imp::VersionsPanel>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl VersionsPanel {
    /// The tab for `target`, which looks versions up through `versions`
    /// once [`Self::load_once`] is called.
    pub(crate) fn new(
        target: &PropertiesTarget,
        versions: Arc<PreviousVersions>,
        locations: LocationContext,
    ) -> Self {
        let panel: Self = glib::Object::new();
        let imp = panel.imp();
        imp.uri.replace(target.uri.clone());
        imp.kind.set(Some(target.kind));
        imp.versions
            .set(versions)
            .expect("a new panel has no service yet");
        imp.locations.replace(locations);
        panel
    }

    fn versions(&self) -> &Arc<PreviousVersions> {
        self.imp().versions.get().expect("new sets the service")
    }

    fn kind(&self) -> ItemKind {
        self.imp().kind.get().unwrap_or(ItemKind::File)
    }

    /// Looks the versions up, the first time only.
    pub(crate) fn load_once(&self) {
        if !self.imp().is_loaded.replace(true) {
            self.load();
        }
    }

    /// Shows the list and starts a new lookup (Refresh).
    pub(crate) fn load(&self) {
        self.cancel();
        self.clear();
        self.append(&quiet_text(VERSIONS_INTRO));
        self.append(&self.toolbar());
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["versions-list"])
            .build();
        list.append(&quiet_text(CHECKING));
        self.append(&list);
        self.imp().list.replace(Some(list));
        self.start_lookup();
    }

    /// Stops a lookup in progress.
    pub(crate) fn cancel(&self) {
        if let Some(lookup) = self.imp().lookup.take() {
            lookup.cancel();
        }
    }

    /// Refresh and Snapshot source….
    fn toolbar(&self) -> gtk::Box {
        let toolbar = gtk::Box::builder().css_classes(["versions-toolbar"]).build();
        let refresh = toolbar_button("Refresh", Icon::ArrowClockwise);
        refresh.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.load()
        ));
        let source = toolbar_button("Snapshot source…", Icon::Settings);
        source.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.show_source_form()
        ));
        toolbar.append(&refresh);
        toolbar.append(&source);
        toolbar
    }

    /// Runs the lookup on a worker thread and shows its result, unless a
    /// newer lookup started meanwhile.
    fn start_lookup(&self) {
        let cancel = Cancellation::new();
        self.imp().lookup.replace(Some(cancel.clone()));
        let generation = self.imp().generation.get().wrapping_add(1);
        self.imp().generation.set(generation);
        let uri = self.imp().uri.borrow().clone();
        let lookup = Arc::clone(self.versions()).find_versions_in_background(uri, self.kind(), cancel);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            async move {
                let result = lookup.await;
                panel.lookup_arrived(generation, result);
            }
        ));
    }

    /// Shows the answer of lookup number `generation`, unless a newer
    /// lookup started meanwhile.
    fn lookup_arrived(&self, generation: u64, result: Result<VersionList, VersionsError>) {
        if self.imp().generation.get() == generation {
            self.imp().lookup.take();
            self.show_result(result);
        }
    }

    /// Shows the versions found, or why there are none.
    fn show_result(&self, result: Result<VersionList, VersionsError>) {
        let Some(list) = self.imp().list.borrow().clone() else {
            return;
        };
        clear_box(&list);
        let found = match result {
            Ok(found) => found,
            Err(VersionsError::Cancelled) => return,
            Err(error) => {
                list.append(&quiet_text(&error.to_string()));
                return;
            }
        };
        if found.versions.is_empty() {
            show_empty(&list, found.message().unwrap_or_default());
        } else {
            self.show_versions(&list, &found);
        }
        self.append_notes(&found);
    }

    /// The date note above the list, the column titles and one row per
    /// version.
    fn show_versions(&self, list: &gtk::Box, found: &VersionList) {
        let date_note = quiet_text(DATE_NOTE);
        date_note.add_css_class("version-date-note");
        self.insert_child_after(&date_note, list.prev_sibling().as_ref());
        list.set_accessible_role(gtk::AccessibleRole::List);
        list.update_property(&[gtk::accessible::Property::Label("Previous versions")]);
        list.append(&column_titles());
        let locations = self.imp().locations.borrow();
        for version in &found.versions {
            list.append(&version_row(version, &locations));
        }
    }

    /// The truncation note, the warnings and the layouts note.
    fn append_notes(&self, found: &VersionList) {
        if found.is_truncated {
            self.append(&quiet_text(TRUNCATED_NOTE));
        }
        if !found.warnings.is_empty() {
            let details = gtk::Expander::builder()
                .label(&ox_core::i18n::gettext("Availability details"))
                .css_classes(["snapshot-warnings"])
                .child(&quiet_text(&found.warnings.join("\n")))
                .build();
            self.append(&details);
        }
        self.append(&note(LAYOUTS_NOTE));
    }

    /// Replaces the list with the Snapshot source form.
    fn show_source_form(&self) {
        self.cancel();
        self.clear();
        let item = SourceItem {
            uri: self.imp().uri.borrow().clone(),
            is_folder: self.kind() == ItemKind::Folder,
        };
        let locations = self.imp().locations.borrow().clone();
        let back = glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move || panel.load()
        );
        let form = gtk::Box::new(gtk::Orientation::Vertical, 0);
        fill_source_form(&form, &item, self.versions(), &locations, back);
        self.append(&form);
    }

    /// Removes everything the tab shows.
    fn clear(&self) {
        clear_box(self.upcast_ref());
        self.imp().list.replace(None);
    }

    /// The number of the newest lookup, for tests.
    #[cfg(test)]
    pub(crate) fn lookup_number(&self) -> u64 {
        self.imp().generation.get()
    }

    /// Delivers `result` as the answer of lookup number `generation`, for
    /// tests.
    #[cfg(test)]
    pub(crate) fn deliver_lookup(&self, generation: u64, result: Result<VersionList, VersionsError>) {
        self.lookup_arrived(generation, result);
    }

    /// Shows the list and starts a new lookup, as Refresh does, for tests.
    #[cfg(test)]
    pub(crate) fn refresh(&self) {
        self.load();
    }

    /// The labels of the versions listed, for tests.
    #[cfg(test)]
    pub(crate) fn version_labels(&self) -> Vec<String> {
        let Some(list) = self.imp().list.borrow().clone() else {
            return Vec::new();
        };
        crate::window::children(&list)
            .filter(|row| row.has_css_class("version-row"))
            .filter_map(|row| {
                crate::test_support::harness::descendants::<gtk::Label>(&row)
                    .into_iter()
                    .next()
            })
            .map(|label| label.text().to_string())
            .collect()
    }

    /// Every text the tab shows, for tests.
    #[cfg(test)]
    pub(crate) fn texts(&self) -> Vec<String> {
        crate::test_support::harness::descendants::<gtk::Label>(self)
            .iter()
            .map(|label| label.text().to_string())
            .collect()
    }
}

/// "Snapshot", "Date & time" and "Actions" above the rows, hidden from
/// screen readers as in app.js: each row names its own parts. The date
/// and actions titles are as wide as those cells of a row.
fn column_titles() -> gtk::Box {
    let titles = gtk::Box::builder().css_classes(["version-columns"]).build();
    let columns = [
        ("Snapshot", -1, true),
        ("Date & time", DATE_COLUMN_WIDTH, false),
        ("Actions", ACTIONS_COLUMN_WIDTH, false),
    ];
    for (title, width, expands) in columns {
        let label = gtk::Label::builder()
            .label(title)
            .xalign(0.0)
            .width_request(width)
            .hexpand(expands)
            .build();
        titles.append(&label);
    }
    titles.set_accessible_role(gtk::AccessibleRole::Presentation);
    titles
}

/// The clock, "No accessible previous versions" and `message`.
fn show_empty(list: &gtk::Box, message: &str) {
    let glyph = icons::image(Icon::Clock, EMPTY_GLYPH);
    glyph.set_halign(gtk::Align::Start);
    list.append(&glyph);
    let heading = gtk::Label::builder()
        .label(NO_VERSIONS)
        .xalign(0.0)
        .css_classes(["versions-empty-heading"])
        .build();
    list.append(&heading);
    list.append(&quiet_text(message));
}

/// A small bordered toolbar button with `glyph` and `label`.
fn toolbar_button(label: &str, glyph: Icon) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    content.append(&icons::image(glyph, TOOLBAR_GLYPH));
    content.append(&gtk::Label::new(Some(label)));
    let button = gtk::Button::builder().child(&content).build();
    button.add_css_class(ButtonStyle::Bordered.css_class());
    button.update_property(&[gtk::accessible::Property::Label(label)]);
    button
}

/// Removes every child of `container`.
fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
