// SPDX-License-Identifier: AGPL-3.0-only
//! The Previous Versions tab (PROP-019), laid out as Windows 11's.
//!
//! "Previous versions come from snapshot and backup folders.", then
//! "File versions:" or "Folder versions:" over a list with Name and Date
//! modified columns, its rows grouped by date ([`super::version_row`]),
//! "Working on it..." while the lookup runs, and Open and Restore under
//! it. Open browses a folder's version in a tab; Restore asks where to put
//! a copy (Restore a copy never overwrites the live item). Snapshot
//! source... on the left of the buttons configures where versions are
//! looked for, and F5 looks again (through [`super::window::PropertiesWindow`]). The lookup starts the first time the
//! tab is shown; looking again, opening Snapshot source... or closing the
//! window cancels a lookup in progress, and a stale lookup's result is
//! ignored.

use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::location::{ItemKind, LocationContext};
use ox_core::transfer::Cancellation;
use ox_core::versions::{PreviousVersions, VersionList, VersionsError};

use super::snapshot_source::{fill_source_form, SourceItem};
use super::version_row::{group_heading, version_row};
use super::window::{paragraph, push_button};
use super::{PropertiesTarget, RestoreRequest, SnapshotTarget};
use crate::dialog::quiet_text;
use crate::icons::{self, Icon};

/// What the tab browses.
const VERSIONS_INTRO: &str =
    crate::i18n::message_id("Previous versions come from snapshot and backup folders.");
/// Shown while the lookup runs, as Windows shows it.
const CHECKING: &str = crate::i18n::message_id("Working on it...");
/// The heading of an empty result, as Windows words it.
const NO_VERSIONS: &str = crate::i18n::message_id("There are no previous versions available.");
/// Under a truncated list.
const TRUNCATED_NOTE: &str = crate::i18n::message_id(
    "Showing at most 100 versions. Configure a narrower snapshot folder to see more.",
);
/// The tooltip of the list: what counts as a previous version.
const LAYOUTS_NOTE: &str = crate::i18n::message_id(
    "Snapshots must already exist and be readable through your NAS. Built-in \
                            layouts include .snapshot, #snapshot, .zfs/snapshot, and Snapper \
                            .snapshots/<id>/snapshot. Restore a copy never overwrites the live item. A \
                            cached search result is not a previous version.",
);
/// The intro's glyph.
const INTRO_GLYPH: i32 = 32;
/// The least height of the list, which grows to fill the tab as Windows'
/// list does; it leaves room for two rows of tabs at the default text
/// size.
const LIST_HEIGHT: i32 = 216;

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
        /// The versions listed, in the list's order, for Open and Restore.
        pub(super) shown: RefCell<Vec<ox_core::versions::PreviousVersion>>,
        /// The rows' list box.
        pub(super) rows: RefCell<Option<gtk::ListBox>>,
        /// Open and Restore.
        pub(super) open: RefCell<Option<gtk::Button>>,
        pub(super) restore: RefCell<Option<gtk::Button>>,
        /// Notes under the list.
        pub(super) notes: RefCell<Option<gtk::Box>>,
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

    /// Lists the versions of the item at `uri`, as after a rename, looking
    /// again if they were shown.
    pub(super) fn set_item_uri(&self, uri: &str) {
        self.imp().uri.replace(uri.to_owned());
        if self.imp().is_loaded.get() {
            self.load();
        }
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

    /// Shows the list and starts a new lookup (F5).
    pub(crate) fn load(&self) {
        self.cancel();
        self.clear();
        let intro = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        intro.add_css_class("versions-intro");
        let glyph = icons::image(Icon::History, INTRO_GLYPH);
        glyph.add_css_class("versions-intro-glyph");
        intro.append(&glyph);
        intro.append(&paragraph(ox_core::i18n::gettext_static(VERSIONS_INTRO)));
        self.append(&intro);
        let caption = if self.kind() == ItemKind::Folder {
            ox_core::i18n::gettext("Folder versions:")
        } else {
            ox_core::i18n::gettext("File versions:")
        };
        let caption = gtk::Label::builder()
            .label(caption)
            .xalign(0.0)
            .css_classes(["properties-caption"])
            .build();
        self.append(&caption);
        let frame = gtk::Frame::builder()
            .css_classes(["properties-list-frame", "versions-frame"])
            .tooltip_text(ox_core::i18n::gettext_static(LAYOUTS_NOTE))
            .build();
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&column_titles());
        let list = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .css_classes(["versions-list"])
            .build();
        list.update_relation(&[gtk::accessible::Relation::LabelledBy(&[caption.upcast_ref()])]);
        let working = quiet_text(ox_core::i18n::gettext_static(CHECKING));
        working.set_halign(gtk::Align::Center);
        working.set_margin_top(24);
        list.append(&working);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .min_content_height(LIST_HEIGHT)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        column.append(&scroller);
        frame.set_child(Some(&column));
        self.append(&frame);
        let notes = gtk::Box::new(gtk::Orientation::Vertical, 0);
        self.append(&notes);
        self.append(&self.buttons());
        let imp = self.imp();
        imp.list.replace(Some(list));
        imp.notes.replace(Some(notes));
        imp.shown.replace(Vec::new());
        imp.rows.replace(None);
        self.update_buttons();
        self.start_lookup();
    }

    /// Stops a lookup in progress.
    pub(crate) fn cancel(&self) {
        if let Some(lookup) = self.imp().lookup.take() {
            lookup.cancel();
        }
    }

    /// Snapshot source... on the left; Open and Restore on the right.
    fn buttons(&self) -> gtk::Box {
        let row = gtk::Box::builder()
            .css_classes(["properties-button-row", "versions-buttons"])
            .build();
        let source = push_button(&ox_core::i18n::gettext("Snapshot source..."));
        source.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.show_source_form()
        ));
        source.set_hexpand(true);
        row.append(&source);
        let open = push_button(&ox_core::i18n::gettext("Open"));
        open.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.run_on_selected(true)
        ));
        let restore = push_button(&ox_core::i18n::gettext("Restore"));
        restore.set_tooltip_text(Some(&ox_core::i18n::gettext(
            "Restore a copy of this version somewhere else; the live item is never overwritten.",
        )));
        restore.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.run_on_selected(false)
        ));
        row.append(&open);
        row.append(&restore);
        self.imp().open.replace(Some(open));
        self.imp().restore.replace(Some(restore));
        row
    }

    /// The version selected in the list.
    fn selected(&self) -> Option<ox_core::versions::PreviousVersion> {
        let imp = self.imp();
        let row = imp.rows.borrow().as_ref()?.selected_row()?;
        let index: usize = row.widget_name().parse().ok()?;
        imp.shown.borrow().get(index).cloned()
    }

    /// Greys Open without a selected folder version and Restore without a
    /// selected version.
    fn update_buttons(&self) {
        let selected = self.selected();
        if let Some(open) = self.imp().open.borrow().as_ref() {
            open.set_sensitive(selected.as_ref().is_some_and(|version| version.entry.is_dir));
        }
        if let Some(restore) = self.imp().restore.borrow().as_ref() {
            restore.set_sensitive(selected.is_some());
        }
    }

    /// Open browses the selected folder version in a tab; Restore asks
    /// where to put a copy of the selected version. Both happen in the
    /// browser window that opened Properties, which comes to the front
    /// first, so what they show is not hidden behind other windows.
    fn run_on_selected(&self, open: bool) {
        let Some(version) = self.selected() else {
            return;
        };
        let Some(opener) = super::window::PropertiesWindow::of(self).and_then(|window| window.opener())
        else {
            return;
        };
        opener.present();
        if open {
            opener.browse_snapshot(SnapshotTarget::of_version(&version));
        } else {
            opener.ask_restore_destination(&RestoreRequest::of_version(&version));
        }
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
                show_empty(&list, &error.to_string());
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

    /// The versions under their date headings.
    fn show_versions(&self, list: &gtk::Box, found: &VersionList) {
        // A click selects a version for Open or Restore; a double click
        // or Enter opens it.
        let rows = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .activate_on_single_click(false)
            .css_classes(["versions-rows"])
            .build();
        rows.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
            "Previous versions",
        ))]);
        let today = glib::DateTime::now_local().ok();
        let locations = self.imp().locations.borrow();
        // Every version is the item itself, as it was: it carries the
        // item's name, as Windows lists it.
        let uri = self.imp().uri.borrow().clone();
        let item_name = gtk::gio::File::for_uri(&uri)
            .basename()
            .map(|name| name.to_string_lossy().into_owned());
        let mut last_heading = None;
        for (index, version) in found.versions.iter().enumerate() {
            let heading = today
                .as_ref()
                .map(|today| group_heading(version.date().as_ref(), today));
            if heading.is_some() && heading != last_heading {
                let label = gtk::Label::builder()
                    .label(heading.unwrap_or_default())
                    .xalign(0.0)
                    .css_classes(["versions-group"])
                    .build();
                let header = gtk::ListBoxRow::builder()
                    .child(&label)
                    .selectable(false)
                    .activatable(false)
                    .build();
                rows.append(&header);
                last_heading = heading;
            }
            let row = gtk::ListBoxRow::builder()
                .child(&version_row(
                    version,
                    item_name.as_deref().unwrap_or(&version.entry.name),
                    &locations,
                ))
                .name(index.to_string())
                .build();
            rows.append(&row);
        }
        rows.connect_selected_rows_changed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.update_buttons()
        ));
        rows.connect_row_activated(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, _| panel.run_on_selected(true)
        ));
        list.append(&rows);
        self.imp().shown.replace(found.versions.clone());
        self.imp().rows.replace(Some(rows));
        self.update_buttons();
    }

    /// The truncation note and the warnings.
    fn append_notes(&self, found: &VersionList) {
        let Some(notes) = self.imp().notes.borrow().clone() else {
            return;
        };
        if found.is_truncated {
            notes.append(&quiet_text(ox_core::i18n::gettext_static(TRUNCATED_NOTE)));
        }
        if !found.warnings.is_empty() {
            let details = gtk::Expander::builder()
                .label(ox_core::i18n::gettext("Availability details"))
                .css_classes(["snapshot-warnings"])
                .child(&quiet_text(&found.warnings.join("\n")))
                .build();
            notes.append(&details);
        }
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
        form.set_vexpand(true);
        fill_source_form(&form, &item, self.versions(), &locations, back);
        self.append(&form);
    }

    /// Removes everything the tab shows.
    fn clear(&self) {
        clear_box(self.upcast_ref());
        let imp = self.imp();
        imp.list.replace(None);
        imp.rows.replace(None);
        imp.open.replace(None);
        imp.restore.replace(None);
        imp.notes.replace(None);
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

    /// Shows the list and starts a new lookup, as F5 does, for tests.
    #[cfg(test)]
    pub(crate) fn refresh(&self) {
        self.load();
    }

    /// The item whose versions are listed, for tests.
    #[cfg(test)]
    pub(crate) fn item_uri(&self) -> String {
        self.imp().uri.borrow().clone()
    }

    /// The snapshot names of the versions listed, for tests.
    #[cfg(test)]
    pub(crate) fn version_labels(&self) -> Vec<String> {
        self.imp()
            .shown
            .borrow()
            .iter()
            .map(|version| version.label.clone())
            .collect()
    }

    /// Selects the version at `index` in the list, for tests.
    #[cfg(test)]
    pub(crate) fn select_version(&self, index: usize) {
        let rows = self.imp().rows.borrow().clone();
        let Some(rows) = rows else {
            return;
        };
        let mut row = rows.first_child();
        while let Some(widget) = row {
            if widget.widget_name() == index.to_string() {
                rows.select_row(widget.downcast_ref::<gtk::ListBoxRow>());
                return;
            }
            row = widget.next_sibling();
        }
    }

    /// Presses Open (`true`) or Restore, for tests.
    #[cfg(test)]
    pub(crate) fn press(&self, open: bool) {
        self.run_on_selected(open);
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

/// "Name" and "Date modified" above the rows, hidden from screen readers:
/// each row names its own parts.
fn column_titles() -> gtk::Box {
    let titles = gtk::Box::builder().css_classes(["version-columns"]).build();
    let name = gtk::Label::builder()
        .label(ox_core::i18n::gettext("Name"))
        .xalign(0.0)
        .hexpand(true)
        .build();
    let date = gtk::Label::builder()
        .label(ox_core::i18n::gettext("Date modified"))
        .xalign(0.0)
        .css_classes(["version-date"])
        .build();
    titles.append(&name);
    titles.append(&date);
    titles.set_accessible_role(gtk::AccessibleRole::Presentation);
    titles
}

/// "There are no previous versions available." and why.
fn show_empty(list: &gtk::Box, message: &str) {
    let heading = gtk::Label::builder()
        .label(ox_core::i18n::gettext_static(NO_VERSIONS))
        .wrap(true)
        .css_classes(["versions-empty-heading"])
        .build();
    heading.set_margin_top(24);
    list.append(&heading);
    if !message.is_empty() {
        let reason = quiet_text(message);
        reason.set_halign(gtk::Align::Center);
        reason.set_justify(gtk::Justification::Center);
        list.append(&reason);
    }
}

/// Removes every child of `container`.
fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
