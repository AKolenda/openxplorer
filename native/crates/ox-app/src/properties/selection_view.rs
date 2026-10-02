// SPDX-License-Identifier: AGPL-3.0-only
//! Properties of several selected items (PROP-002), as Dolphin's
//! `KPropertiesDialog(KFileItemList)` and Explorer's Properties of a
//! multiple selection show them: a General tab with how many files and
//! folders are selected, their common type and folder, and their combined
//! size and content, which the folders' sizes are measured for; and a
//! Permissions tab whose change applies to all of them, when the user owns
//! every one, and keeps every bit the user did not change.

use gtk::glib;
use gtk::prelude::*;
use ox_core::entry::Entry;
use ox_core::format;
use ox_core::location::parent_location;
use ox_core::permissions::Account;
use ox_core::sizes::{scan_folder_size_in_background, ScanStatus};
use ox_core::transfer::Cancellation;

use super::folder_sizes::counts_text;
use super::metadata::{read_properties, ItemProperties};
use super::permissions_editor::{permissions_editor, EditedItems};
use super::tabs::PropertiesTabs;
use super::view::{can_edit_permissions, PropertiesContext};
use super::{PropertiesTab, CALCULATING, READING};
use crate::dialog::{note, quiet_text, PropertyGrid};
use crate::icons::{Art, ArtImage};

/// The Permissions tab when some item cannot be changed here.
const NOT_EDITABLE: &str = "Permissions can be changed together only for items you own, outside previous \
                            versions and shares.";

/// The General and Permissions tabs of a multiple selection.
#[derive(Debug, Clone)]
pub(crate) struct SelectionProperties {
    root: gtk::Box,
    cancel: Cancellation,
}

impl SelectionProperties {
    /// The Properties of `entries`, which are at least two, opened on
    /// General or Permissions as `initial` says. It starts measuring the
    /// folders and reading the items' permissions at once.
    pub(crate) fn new(entries: Vec<Entry>, context: &PropertiesContext, initial: PropertiesTab) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("properties-view");
        let tabs = PropertiesTabs::new();
        root.append(tabs.tab_row());
        root.append(tabs.pages());
        let general = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let permissions = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tabs.add_page(PropertiesTab::General, &general);
        tabs.add_page(PropertiesTab::Permissions, &permissions);
        tabs.select_tab(initial);
        let cancel = Cancellation::new();
        let size = fill_general(&general, &entries, context);
        measure(&entries, &size, &cancel);
        permissions.append(&quiet_text(READING));
        read_permissions(&permissions, entries, context.clone());
        Self { root, cancel }
    }

    /// The dialog's title: `<count> items Properties`.
    pub(crate) fn dialog_title(count: usize) -> String {
        format!("{count} items Properties")
    }

    /// The widget, for the dialog's body.
    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Stops measuring the folders.
    pub(crate) fn cancel_work(&self) {
        self.cancel.cancel();
    }
}

/// The Size and Contains values, which measuring the folders updates.
#[derive(Debug, Clone)]
struct SizeRows {
    size: gtk::Label,
    contains: gtk::Label,
}

/// Fills the General tab and returns its Size and Contains values.
fn fill_general(panel: &gtk::Box, entries: &[Entry], context: &PropertiesContext) -> SizeRows {
    let files = entries.iter().filter(|entry| !entry.is_dir).count();
    let folders = entries.len() - files;
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["property-file"])
        .build();
    if let Some(first) = entries.first() {
        header.append(&ArtImage::new(Art::for_entry(first), 48));
    }
    let heading = gtk::Label::builder()
        .label(counts_text(files as u64, folders as u64))
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["property-name-heading"])
        .build();
    header.append(&heading);
    panel.append(&header);
    let grid = PropertyGrid::new();
    grid.add_row(&ox_core::i18n::gettext("Type"), &common_type(entries));
    grid.add_row(
        &ox_core::i18n::gettext("Location"),
        &common_location(entries, context),
    );
    let bytes: u64 = entries
        .iter()
        .filter_map(|entry| entry.size.filter(|_| !entry.is_dir))
        .sum();
    let has_folders = folders > 0;
    let size = grid.add_row(
        &ox_core::i18n::gettext("Size"),
        &if has_folders {
            CALCULATING.to_owned()
        } else {
            format::pretty_bytes(bytes)
        },
    );
    let contains = grid.add_row(
        &ox_core::i18n::gettext("Contains"),
        &if has_folders {
            CALCULATING.to_owned()
        } else {
            counts_text(files as u64, 0)
        },
    );
    panel.append(grid.widget());
    SizeRows { size, contains }
}

/// The items' type when they share one, else "Multiple types".
fn common_type(entries: &[Entry]) -> String {
    let first = entries
        .first()
        .map(|entry| entry.type_label.as_str())
        .unwrap_or_default();
    if entries.iter().all(|entry| entry.type_label == first) {
        format!("All of type {first}")
    } else {
        "Multiple types".to_owned()
    }
}

/// "All in <folder>" when the items share a folder, else "Multiple
/// locations".
fn common_location(entries: &[Entry], context: &PropertiesContext) -> String {
    let first = entries.first().and_then(|entry| parent_location(&entry.uri));
    let shared = entries.iter().all(|entry| parent_location(&entry.uri) == first);
    match first.filter(|_| shared) {
        Some(folder) => format!("All in {}", context.locations.display_location(&folder)),
        None => "Multiple locations".to_owned(),
    }
}

/// Measures the selected folders one after another and shows the
/// combined size and content once every one is measured.
fn measure(entries: &[Entry], rows: &SizeRows, cancel: &Cancellation) {
    let folders: Vec<String> = entries
        .iter()
        .filter(|entry| entry.is_dir)
        .map(|entry| entry.uri.clone())
        .collect();
    if folders.is_empty() {
        return;
    }
    let mut bytes: u64 = entries
        .iter()
        .filter_map(|entry| entry.size.filter(|_| !entry.is_dir))
        .sum();
    let mut files = entries.iter().filter(|entry| !entry.is_dir).count() as u64;
    let mut subfolders = folders.len() as u64;
    let rows = rows.clone();
    let cancel = cancel.clone();
    glib::spawn_future_local(async move {
        let mut is_complete = true;
        for uri in folders {
            let noop = |_: &ox_core::sizes::FolderSize| {};
            match scan_folder_size_in_background(uri, cancel.clone(), noop).await {
                Ok(size) => {
                    bytes += size.bytes;
                    files += size.files;
                    subfolders += size.folders;
                    is_complete &= size.status == ScanStatus::Complete;
                }
                Err(_) => is_complete = false,
            }
            if cancel.is_cancelled() {
                return;
            }
        }
        let at_least = if is_complete { "" } else { "≥ " };
        rows.size
            .set_text(&format!("{at_least}{}", format::pretty_bytes(bytes)));
        rows.contains
            .set_text(&format!("{at_least}{}", counts_text(files, subfolders)));
    });
}

/// Reads every item's permissions, then fills the Permissions tab: the
/// items' owner and group, and the editor when every item can be
/// changed here.
fn read_permissions(panel: &gtk::Box, entries: Vec<Entry>, context: PropertiesContext) {
    let panel = panel.downgrade();
    glib::spawn_future_local(async move {
        let mut items: Vec<ItemProperties> = Vec::new();
        for entry in entries {
            let Ok(properties) = read_properties(entry.navigation_uri().to_owned()).await else {
                items.clear();
                break;
            };
            items.push(properties);
        }
        let Some(panel) = panel.upgrade() else {
            return;
        };
        while let Some(child) = panel.first_child() {
            panel.remove(&child);
        }
        let Some(first) = items.first() else {
            panel.append(&note(NOT_EDITABLE));
            return;
        };
        let grid = PropertyGrid::new();
        let same = |value: fn(&ItemProperties) -> Option<&str>| {
            let first = value(first);
            if items.iter().all(|item| value(item) == first) {
                first.unwrap_or_default().to_owned()
            } else {
                "Multiple".to_owned()
            }
        };
        grid.add_row(
            &ox_core::i18n::gettext("Owner"),
            &same(|item| item.owner.as_deref()),
        );
        grid.add_row(
            &ox_core::i18n::gettext("Group"),
            &same(|item| item.group.as_deref()),
        );
        panel.append(grid.widget());
        if !items.iter().all(|item| can_edit_permissions(item, &context)) {
            panel.append(&note(NOT_EDITABLE));
            return;
        }
        let shared = |account: fn(&ItemProperties) -> Option<Account>| {
            let first = account(first);
            items
                .iter()
                .all(|item| account(item) == first)
                .then_some(first)
                .flatten()
        };
        let edited = EditedItems {
            items: items.iter().map(ItemProperties::edited_item).collect(),
            owner: shared(ItemProperties::owner_account),
            group: shared(ItemProperties::group_account),
        };
        panel.append(&permissions_editor(
            edited,
            std::sync::Arc::clone(&context.versions),
        ));
    });
}
