// SPDX-License-Identifier: AGPL-3.0-only
//! The "Add folders to the index" table: folders the index does not keep
//! yet, why each is suggested and where it is, to index one at a time or
//! several at once.
//!
//! Ports the unchecked rows of `renderSettingsCache` in
//! `v2.0.0:desktop/ui/app.js` (SET-006) as the settings mockup's power-user table:
//! a check box per folder, the folder's picture, name and path, the reason
//! it is suggested, its location, and an Index button; under the table,
//! how many are selected, Clear and "Index selected". [`IndexSuggestions`]
//! is a `GtkBox` subclass that keeps which folders are selected while the
//! list is refreshed.

use std::collections::BTreeSet;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

use super::commands::IndexCommand;
use super::IndexCandidate;
use crate::icons::{Art, ArtImage};
use crate::settings_page::{parts, SettingsPage};
use crate::window::ButtonStyle;

/// A folder's picture in the table (`small16` in the mockup, a little
/// larger to match the rows' text).
const FOLDER_ART: i32 = 20;

/// The table's columns, as its heading names them.
const COLUMN_TITLES: [&str; 3] = ["Folder", "Suggested because", "Location"];

mod imp {
    use std::cell::{OnceCell, RefCell};
    use std::collections::BTreeSet;

    use gtk::glib;
    use gtk::subclass::prelude::*;

    use super::super::IndexCandidate;
    use crate::settings_page::SettingsPage;

    /// Private state of [`super::IndexSuggestions`].
    #[derive(Debug, Default)]
    pub(crate) struct IndexSuggestions {
        /// The heading and one line per folder.
        pub(super) grid: gtk::Grid,
        /// "2 selected", under the table.
        pub(super) selection_label: gtk::Label,
        /// "Index selected".
        pub(super) index_selected: gtk::Button,
        /// The folders listed now.
        pub(super) listed: RefCell<Vec<IndexCandidate>>,
        /// The URIs of the folders whose boxes are checked.
        pub(super) selected: RefCell<BTreeSet<String>>,
        /// The page that runs the table's commands.
        pub(super) page: OnceCell<glib::WeakRef<SettingsPage>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for IndexSuggestions {
        const NAME: &'static str = "OxIndexSuggestions";
        type Type = super::IndexSuggestions;
        type ParentType = gtk::Box;
    }

    impl ObjectImpl for IndexSuggestions {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for IndexSuggestions {}
    impl BoxImpl for IndexSuggestions {}
}

glib::wrapper! {
    /// The table of folders to add to the index.
    pub(crate) struct IndexSuggestions(ObjectSubclass<imp::IndexSuggestions>)
        @extends gtk::Box, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Orientable;
}

impl IndexSuggestions {
    /// An empty table whose buttons ask `page` to index folders.
    pub(crate) fn new(page: &SettingsPage) -> Self {
        let table: Self = glib::Object::new();
        table
            .imp()
            .page
            .set(page.downgrade())
            .expect("a new table has no page yet");
        table
    }

    /// The frame, the grid and the footer.
    fn build(&self) {
        self.set_orientation(gtk::Orientation::Vertical);
        self.add_css_class("index-suggestions");
        let imp = self.imp();
        imp.grid.set_column_spacing(16);
        imp.grid.set_row_spacing(8);
        imp.grid.add_css_class("suggestion-grid");
        self.append(&imp.grid);
        self.append(&self.footer());
    }

    /// "N selected", Clear and "Index selected".
    fn footer(&self) -> gtk::Box {
        let imp = self.imp();
        let footer = gtk::Box::builder()
            .spacing(12)
            .css_classes(["suggestion-footer"])
            .build();
        imp.selection_label.set_xalign(0.0);
        imp.selection_label.set_hexpand(true);
        footer.append(&imp.selection_label);
        let clear = gtk::Button::builder()
            .label(ox_core::i18n::gettext("Clear"))
            .valign(gtk::Align::Center)
            .css_classes(["link-button"])
            .build();
        clear.connect_clicked(glib::clone!(
            #[weak(rename_to = table)]
            self,
            move |_| table.clear_selection()
        ));
        footer.append(&clear);
        imp.index_selected
            .set_label(&ox_core::i18n::gettext("Index selected"));
        imp.index_selected.add_css_class(ButtonStyle::Accent.css_class());
        imp.index_selected.connect_clicked(glib::clone!(
            #[weak(rename_to = table)]
            self,
            move |_| table.index_selected()
        ));
        footer.append(&imp.index_selected);
        footer
    }

    /// Lists `candidates`, keeping the selection of those still listed.
    pub(crate) fn show(&self, candidates: &[IndexCandidate]) {
        let imp = self.imp();
        if *imp.listed.borrow() == candidates {
            return;
        }
        imp.listed.replace(candidates.to_vec());
        imp.selected
            .borrow_mut()
            .retain(|uri| candidates.iter().any(|candidate| &candidate.uri == uri));
        self.fill_grid();
        self.show_selection_count();
    }

    /// The names of the folders listed, for tests.
    #[cfg(test)]
    pub(crate) fn listed_labels(&self) -> Vec<String> {
        let listed = self.imp().listed.borrow();
        listed.iter().map(|candidate| candidate.label.clone()).collect()
    }

    /// Rebuilds the heading and one line per listed folder.
    fn fill_grid(&self) {
        let grid = &self.imp().grid;
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }
        for (column, title) in (1..).zip(COLUMN_TITLES) {
            let heading = gtk::Label::builder()
                .label(title)
                .xalign(0.0)
                .css_classes(["suggestion-heading"])
                .build();
            grid.attach(&heading, column, 0, 1, 1);
        }
        let listed = self.imp().listed.borrow().clone();
        for (line, candidate) in (1..).zip(&listed) {
            self.attach_line(line, candidate);
        }
    }

    /// Puts `candidate`'s check box, folder, reason, location and Index
    /// button on grid line `line`.
    fn attach_line(&self, line: i32, candidate: &IndexCandidate) {
        let grid = &self.imp().grid;
        grid.attach(&self.check_box(candidate), 0, line, 1, 1);
        grid.attach(&folder_cell(candidate), 1, line, 1, 1);
        grid.attach(&muted_label(candidate.reason.text()), 2, line, 1, 1);
        grid.attach(&muted_label(&candidate.location), 3, line, 1, 1);
        grid.attach(&self.index_button(candidate), 4, line, 1, 1);
    }

    /// The box that selects `candidate`, named "Cache <label>" as the
    /// Python check box was.
    fn check_box(&self, candidate: &IndexCandidate) -> gtk::CheckButton {
        let is_selected = self.imp().selected.borrow().contains(&candidate.uri);
        let check = gtk::CheckButton::builder()
            .active(is_selected)
            .valign(gtk::Align::Center)
            .build();
        let name = ox_core::i18n::format_message("Cache {label}", &[("label", &candidate.label)]);
        check.update_property(&[gtk::accessible::Property::Label(&name)]);
        let uri = candidate.uri.clone();
        check.connect_toggled(glib::clone!(
            #[weak(rename_to = table)]
            self,
            move |check| table.select(&uri, check.is_active())
        ));
        check
    }

    /// The Index button of `candidate`'s line.
    fn index_button(&self, candidate: &IndexCandidate) -> gtk::Button {
        let button = parts::button(&ox_core::i18n::gettext("Index"), ButtonStyle::Bordered);
        let name = ox_core::i18n::format_message("Index {label}", &[("label", &candidate.label)]);
        button.update_property(&[gtk::accessible::Property::Label(&name)]);
        let command = candidate.index_command();
        button.connect_clicked(glib::clone!(
            #[weak(rename_to = table)]
            self,
            move |_| table.run(command.clone())
        ));
        button
    }

    /// Adds `uri` to the selection, or takes it out.
    fn select(&self, uri: &str, is_selected: bool) {
        let mut selected = self.imp().selected.borrow_mut();
        if is_selected {
            selected.insert(uri.to_owned());
        } else {
            selected.remove(uri);
        }
        drop(selected);
        self.show_selection_count();
    }

    /// Unchecks every box.
    fn clear_selection(&self) {
        self.imp().selected.borrow_mut().clear();
        self.fill_grid();
        self.show_selection_count();
    }

    /// Indexes every selected folder.
    fn index_selected(&self) {
        let selected: BTreeSet<String> = self.imp().selected.take();
        let listed = self.imp().listed.borrow().clone();
        let chosen = listed
            .iter()
            .filter(|candidate| selected.contains(&candidate.uri));
        for candidate in chosen {
            self.run(candidate.index_command());
        }
        self.show_selection_count();
    }

    /// "2 selected", and "Index selected" only with a selection.
    fn show_selection_count(&self) {
        let imp = self.imp();
        let count = imp.selected.borrow().len();
        imp.selection_label.set_text(&ox_core::i18n::format_message(
            "{count} selected",
            &[("count", &count.to_string())],
        ));
        imp.index_selected.set_sensitive(count > 0);
    }

    /// Hands `command` to the page.
    fn run(&self, command: IndexCommand) {
        let page = self.imp().page.get().and_then(glib::WeakRef::upgrade);
        if let Some(page) = page {
            page.run_index_command(command);
        }
    }

    /// Checks the box of the folder labelled `label`, for tests.
    #[cfg(test)]
    pub(crate) fn check(&self, label: &str) {
        let position = self.listed_labels().iter().position(|listed| listed == label);
        let position = position.expect("the table lists the folder");
        let boxes = crate::test_support::harness::descendants::<gtk::CheckButton>(self);
        boxes[position].set_active(true);
    }

    /// Clicks "Index selected", for tests.
    #[cfg(test)]
    pub(crate) fn click_index_selected(&self) {
        self.imp().index_selected.emit_clicked();
    }
}

/// The folder's picture, name and path.
fn folder_cell(candidate: &IndexCandidate) -> gtk::Box {
    let cell = gtk::Box::builder().spacing(10).hexpand(true).build();
    let art = if candidate.is_network {
        Art::SHARE
    } else {
        Art::Folder
    };
    cell.append(&ArtImage::new(art, FOLDER_ART));
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
    texts.append(&parts::wrapped_label(&candidate.label, "setting-title"));
    texts.append(&parts::wrapped_label(&candidate.path, "setting-description"));
    cell.append(&texts);
    cell
}

/// A muted cell of the table.
fn muted_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .valign(gtk::Align::Center)
        .css_classes(["setting-description"])
        .build()
}
