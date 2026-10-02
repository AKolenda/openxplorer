// SPDX-License-Identifier: AGPL-3.0-only
//! The Indexed folders page: the folders the search index keeps, and the
//! folders to add to it.
//!
//! Ports `renderSettingsCache` and the "Add a folder" field of
//! `renderSettingsPage` in `v2.0.0:desktop/ui/app.js` (SET-006, SET-007) in the
//! layout of the settings mockup. The Python app listed every candidate
//! folder in one long list with a check box each; here the page opens from
//! the "Folders to index" row and shows two groups. "Indexed folders"
//! lists the folders the index keeps, each with its state and buttons
//! ([`root_row`]). "Add folders to the index" has the path field and the
//! table of the other candidates ([`suggestions`]), which offers the same
//! folders as Python, once each: the folder shown before Settings opened,
//! the folders indexed before, Home, Quick access, saved shares, the Local
//! Disk and mounted drives, never pages, devices or SMB servers. Every
//! button runs an [`IndexCommand`] ([`commands`]).

mod candidates;
mod commands;
mod root_row;
mod suggestions;

use std::cell::RefCell;

use gtk::glib;
use gtk::prelude::*;
use ox_core::location::{same_location, LocationContext};
use ox_core::search::IndexRoot;

pub(crate) use candidates::{index_candidates, CandidateSources, IndexCandidate};
pub(crate) use commands::IndexCommand;
pub(crate) use root_row::grouped_number;
use suggestions::IndexSuggestions;

use super::group::SettingsGroup;
use super::parts;
use super::row::{ControlName, RowLayout, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::SettingsPage;
use crate::icons::Icon;
use crate::window::ButtonStyle;

/// The page's line under its title: the Python section's help text.
const LEAD: &str = crate::i18n::message_id(
    "Check a folder to index the names and paths of its files and subfolders. SMB \
                    folders work too. File contents are never cached. Open a protected share and \
                    sign in before indexing it.",
);

/// The limits of indexing, from the Python section's help.
const LIMITS_NOTE: &str = crate::i18n::message_id(
    "Local changes update the index after a short debounce. Watching uses up \
                           to 8,192 directories; timed checks cover any remaining ones. Disk roots \
                           skip system/temporary folders, nested mounts and symlinks. Select each \
                           mounted volume separately. Initial scans are limited to 1 million \
                           entries.",
);

/// What the "Indexed folders" group says while the index keeps nothing.
const NOTHING_INDEXED: &str =
    crate::i18n::message_id("No folders are indexed yet. Add one below, or pin a folder to Quick access.");

const ADD_FOLDER: RowText = RowText {
    title: "Add a folder",
    description: "A path relative to the folder you came from, or a share such as \\\\nas\\share.",
    keywords: "custom directory path local disk smb nas",
};

/// An indexed folder as its row shows it: the root and how its folder is
/// named and placed.
type IndexedRow = (IndexRoot, IndexCandidate);

/// The Indexed folders page's lists, which the window refills whenever
/// the places or the cache status change.
#[derive(Debug)]
pub(super) struct IndexedFolders {
    /// The folders the index keeps.
    indexed: SettingsGroup,
    /// The folders to add.
    suggestions: IndexSuggestions,
    /// What the indexed list shows now, so an unchanged status rebuilds
    /// nothing.
    shown: RefCell<Vec<IndexedRow>>,
}

impl IndexedFolders {
    /// Shows the enabled `roots` as indexed folders and the other
    /// `candidates` in the table; their buttons run commands on `page`.
    pub(super) fn show(&self, candidates: &[IndexCandidate], roots: &[IndexRoot], page: &SettingsPage) {
        let enabled: Vec<&IndexRoot> = roots.iter().filter(|root| root.is_enabled()).collect();
        let is_indexed = |uri: &str| enabled.iter().any(|root| same_location(&root.uri, uri));
        let suggested: Vec<IndexCandidate> = candidates
            .iter()
            .filter(|candidate| !is_indexed(&candidate.uri))
            .cloned()
            .collect();
        self.suggestions.show(&suggested);
        let indexed: Vec<IndexedRow> = enabled
            .into_iter()
            .map(|root| (root.clone(), candidate_of(root, candidates)))
            .collect();
        if *self.shown.borrow() != indexed {
            self.show_indexed(&indexed, page);
            self.shown.replace(indexed);
        }
    }

    /// Rebuilds the rows of "Indexed folders".
    fn show_indexed(&self, indexed: &[IndexedRow], page: &SettingsPage) {
        self.indexed.remove_rows();
        if indexed.is_empty() {
            show_nothing_indexed(&self.indexed);
        }
        for (root, candidate) in indexed {
            let row = root_row::root_row(root, &candidate.path, candidate.is_network, page);
            self.indexed.add_plain_row(&row);
        }
    }

    /// The table of folders to add, for tests.
    #[cfg(test)]
    pub(super) fn suggestions(&self) -> &IndexSuggestions {
        &self.suggestions
    }

    /// The names of the indexed folders, then of the folders to add, as
    /// the page lists them.
    #[cfg(test)]
    pub(super) fn shown_labels(&self) -> Vec<String> {
        let shown = self.shown.borrow();
        let indexed = shown.iter().map(|(root, _)| root.label.clone());
        indexed.chain(self.suggestions.listed_labels()).collect()
    }
}

/// How `root` is listed: as the candidate for its folder, or else by its
/// own label and display path.
fn candidate_of(root: &IndexRoot, candidates: &[IndexCandidate]) -> IndexCandidate {
    let listed = candidates
        .iter()
        .find(|candidate| same_location(&candidate.uri, &root.uri));
    let mut candidate = listed
        .cloned()
        .unwrap_or_else(|| candidates::indexed_root(root, &LocationContext::default()));
    candidate.label.clone_from(&root.label);
    candidate
}

/// The Indexed folders page and its lists.
pub(super) fn build(page: &SettingsPage) -> (SettingsSection, IndexedFolders) {
    let section = SettingsSection::new(
        &ox_core::i18n::gettext("Indexed folders"),
        ox_core::i18n::gettext_static(LEAD),
        PageKind::Subpage,
    );
    let indexed = SettingsGroup::new(&ox_core::i18n::gettext("Indexed folders"));
    // The list starts empty, so it says so until the index reports.
    show_nothing_indexed(&indexed);
    section.append_group(&indexed);
    let add_group = SettingsGroup::new(&ox_core::i18n::gettext("Add folders to the index"));
    let add_field = add_folder_field();
    add_group.add_row(&add_folder_row(&add_field, page));
    section.append_group(&add_group);
    let suggestions = IndexSuggestions::new(page);
    section.append_text(&suggestions);
    section.append_text(&parts::note(
        Icon::Info,
        ox_core::i18n::gettext_static(LIMITS_NOTE),
    ));
    let folders = IndexedFolders {
        indexed,
        suggestions,
        shown: RefCell::default(),
    };
    (section, folders)
}

/// Says in `group` that the index keeps no folder.
fn show_nothing_indexed(group: &SettingsGroup) {
    let empty = parts::wrapped_label(
        ox_core::i18n::gettext_static(NOTHING_INDEXED),
        "setting-description",
    );
    empty.add_css_class("empty-group");
    group.add_plain_row(&empty);
}

/// The path field of "Add a folder" (SET-007): wide, and never cutting
/// off what was typed.
fn add_folder_field() -> gtk::Entry {
    let field = gtk::Entry::builder()
        .placeholder_text(ox_core::i18n::gettext(
            "Add a folder: /home/you/Projects or \\\\nas\\share",
        ))
        .hexpand(true)
        .width_chars(36)
        .build();
    field.update_property(&[gtk::accessible::Property::Label(&ox_core::i18n::gettext(
        "Folder to cache",
    ))]);
    field
}

/// "Add a folder": the path field and "Index", which indexes the folder
/// typed, relative to the folder shown before Settings, and empties the
/// field (SET-007).
fn add_folder_row(field: &gtk::Entry, page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(ADD_FOLDER);
    row.add_control(field, ControlName::RowTitle);
    let index = parts::button(&ox_core::i18n::gettext("Index"), ButtonStyle::Accent);
    row.add_control(&index, ControlName::OwnLabel);
    field.connect_activate(glib::clone!(
        #[weak]
        page,
        move |field| page.index_typed_folder(field)
    ));
    index.connect_clicked(glib::clone!(
        #[weak]
        page,
        #[weak]
        field,
        move |_| page.index_typed_folder(&field)
    ));
    row.set_roomy_layout(RowLayout::ControlsBelow);
    row
}

impl SettingsPage {
    /// Indexes the folder typed into `field` and empties it.
    fn index_typed_folder(&self, field: &gtk::Entry) {
        let typed = field.text().to_string();
        let base = self.index_origin();
        self.run_index_command(IndexCommand::IndexTyped { typed, base });
        field.set_text("");
    }
}
