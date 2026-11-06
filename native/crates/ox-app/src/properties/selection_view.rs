// SPDX-License-Identifier: AGPL-3.0-only
//! Properties of several selected items (PROP-002), as Windows 11's
//! Properties of a multiple selection show them: a General tab with how
//! many files and folders are selected, their common type and folder, and
//! their combined size, size on disk and content, counting up as the
//! folders are measured; and a Security tab whose change applies to all of
//! them, when the user owns every one, and keeps every bit the user did
//! not change.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::Entry;
use ox_core::format;
use ox_core::location::parent_location;
use ox_core::permissions::Account;
use ox_core::transfer::Cancellation;

use super::folder_count::{count_folders, FolderCountRows, Totals};
use super::item_window::{can_edit_permissions, PropertiesContext};
use super::metadata::{read_properties, ItemProperties};
use super::permissions_editor::{permissions_editor, EditedItems};
use super::window::{divider, page, PropertiesWindow, Rows};
use super::{PropertiesTab, READING};
use crate::dialog::{note, quiet_text, PropertyGrid};
use crate::icons::{Art, ArtImage};
use crate::window::BrowserWindow;

/// The Security tab when some item cannot be changed here.
const NOT_EDITABLE: &str = crate::i18n::message_id(
    "Permissions can be changed together only for items you own, outside previous \
                            versions and shares.",
);

/// The title of a selection's Properties: `<count> items Properties`.
pub(crate) fn selection_title(count: usize) -> String {
    ox_core::i18n::format_message(
        "{count} items Properties",
        &[("count", &ox_core::format::group_digits(count as u64))],
    )
}

/// Opens the Properties of `entries`, which are at least two, on General
/// or Security as `initial` says. It starts measuring the folders and
/// reading the items' permissions at once.
pub(crate) fn open_selection_properties(
    opener: &BrowserWindow,
    entries: Vec<Entry>,
    context: &PropertiesContext,
    initial: PropertiesTab,
) -> PropertiesWindow {
    let art = entries.first().map(Art::for_entry);
    let window = PropertiesWindow::new(&selection_title(entries.len()), art, Some(opener));
    let general = page();
    let security = page();
    window.add_tab(
        PropertiesTab::General.page_name(),
        PropertiesTab::General.label(),
        &general,
    );
    window.add_tab(
        PropertiesTab::Security.page_name(),
        PropertiesTab::Security.label(),
        &security,
    );
    window.select_tab(initial.page_name());
    let cancel = Cancellation::new();
    window.track_work(cancel.clone());
    let rows = fill_general(&general, &entries, context);
    measure(&entries, &rows, &cancel);
    security.append(&quiet_text(ox_core::i18n::gettext_static(READING)));
    read_permissions(&security, entries, context.clone(), window.downgrade());
    window
}

/// Fills the General tab and returns its Size, Size on disk and Contains
/// values.
fn fill_general(panel: &gtk::Box, entries: &[Entry], context: &PropertiesContext) -> FolderCountRows {
    let files = entries.iter().filter(|entry| !entry.is_dir).count() as u64;
    let folders = entries.len() as u64 - files;
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .css_classes(["properties-header"])
        .build();
    if let Some(first) = entries.first() {
        header.append(&ArtImage::new(Art::for_entry(first), 32));
    }
    let heading = gtk::Label::builder()
        .label(format::contents_text(files, folders))
        .xalign(0.0)
        .hexpand(true)
        .selectable(true)
        .build();
    header.append(&heading);
    panel.append(&header);
    panel.append(&divider());
    let kind = Rows::new();
    kind.add(&ox_core::i18n::gettext("Type"), &common_type(entries));
    kind.add_line(
        &ox_core::i18n::gettext("Location"),
        &common_location(entries, context),
    );
    panel.append(kind.widget());
    panel.append(&divider());
    let sizes = Rows::new();
    let size = sizes.add(&ox_core::i18n::gettext("Size"), "");
    let on_disk = sizes.add(&ox_core::i18n::gettext("Size on disk"), "");
    let contains = sizes.add(&ox_core::i18n::gettext("Contains"), "");
    panel.append(sizes.widget());
    FolderCountRows {
        size,
        on_disk,
        contains,
    }
}

/// The items' type when they share one, else "Multiple types".
fn common_type(entries: &[Entry]) -> String {
    let first = entries
        .first()
        .map(|entry| entry.type_label.as_str())
        .unwrap_or_default();
    if entries.iter().all(|entry| entry.type_label == first) {
        ox_core::i18n::format_message("All of type {first}", &[("first", first)])
    } else {
        ox_core::i18n::gettext_static("Multiple types").to_owned()
    }
}

/// "All in <folder>" when the items share a folder, else "Multiple
/// locations".
fn common_location(entries: &[Entry], context: &PropertiesContext) -> String {
    let first = entries.first().and_then(|entry| parent_location(&entry.uri));
    let shared = entries.iter().all(|entry| parent_location(&entry.uri) == first);
    match first.filter(|_| shared) {
        Some(folder) => ox_core::i18n::format_message(
            "All in {display_location}",
            &[("display_location", &context.locations.display_location(&folder))],
        ),
        None => ox_core::i18n::gettext_static("Multiple locations").to_owned(),
    }
}

/// Reads the space each selected file takes, then counts the selected
/// folders one after another, showing the totals as they climb.
fn measure(entries: &[Entry], rows: &FolderCountRows, cancel: &Cancellation) {
    let folders: Vec<String> = entries
        .iter()
        .filter(|entry| entry.is_dir)
        .map(|entry| entry.uri.clone())
        .collect();
    let files: Vec<(String, u64)> = entries
        .iter()
        .filter(|entry| !entry.is_dir)
        .map(|entry| (entry.uri.clone(), entry.size.unwrap_or_default()))
        .collect();
    let rows = rows.clone();
    let cancel = cancel.clone();
    glib::spawn_future_local(async move {
        let mut totals = Totals::zero();
        for (uri, bytes) in files {
            totals.bytes = totals.bytes.saturating_add(bytes);
            totals.files += 1;
            totals.on_disk = totals
                .on_disk
                .zip(size_on_disk(&uri, &cancel).await)
                .map(|(total, allocated)| total.saturating_add(allocated));
            if cancel.is_cancelled() {
                return;
            }
        }
        totals.folders = folders.len() as u64;
        count_folders(folders, totals, &rows, &cancel);
    });
}

/// The bytes the file at `uri` takes on disk; for a link, the file it
/// points to, as the Size row counts it (a listed link has its target's
/// size). `None` when it cannot be read, its backend does not say, or
/// `cancel` stops the reading, as closing the dialog does.
async fn size_on_disk(uri: &str, cancel: &Cancellation) -> Option<u64> {
    let (sender, receiver) = async_channel::bounded(1);
    gio::File::for_uri(uri).query_info_async(
        "standard::allocated-size",
        gio::FileQueryInfoFlags::NONE,
        glib::Priority::DEFAULT,
        Some(cancel.cancellable()),
        move |result| {
            let _ = sender.try_send(result);
        },
    );
    let info = receiver.recv().await.ok()?.ok()?;
    info.has_attribute("standard::allocated-size")
        .then(|| info.attribute_uint64("standard::allocated-size"))
}

/// Reads every item's permissions, then fills the Security tab: the
/// items' owner and group, and the editor when every item can be
/// changed here.
fn read_permissions(
    panel: &gtk::Box,
    entries: Vec<Entry>,
    context: PropertiesContext,
    window: glib::WeakRef<PropertiesWindow>,
) {
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
            panel.append(&note(ox_core::i18n::gettext_static(NOT_EDITABLE)));
            return;
        };
        let grid = PropertyGrid::new();
        let same = |value: fn(&ItemProperties) -> Option<&str>| {
            let first = value(first);
            if items.iter().all(|item| value(item) == first) {
                first.unwrap_or_default().to_owned()
            } else {
                ox_core::i18n::gettext("Multiple")
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
            panel.append(&note(ox_core::i18n::gettext_static(NOT_EDITABLE)));
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
        let (editor, change) = permissions_editor(edited, std::sync::Arc::clone(&context.versions));
        panel.append(&editor);
        // The window's OK and Apply make the editor's change too.
        if let Some(window) = window.upgrade() {
            window.add_pending(change);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closing Properties cancels the reading of each item's space on
    /// disk too, so a slow backend is not left answering a closed dialog.
    ///
    /// parity: PROP-033
    #[gtk::test]
    fn a_cancelled_dialog_stops_reading_the_space_on_disk() {
        let folder = tempfile::tempdir().expect("a folder");
        let file = folder.path().join("notes.txt");
        std::fs::write(&file, vec![1_u8; 10_000]).expect("the file is written");
        let uri = gio::File::for_path(&file).uri().to_string();
        let open = Cancellation::new();
        let read = glib::MainContext::default().block_on(size_on_disk(&uri, &open));
        assert!(read.is_some_and(|bytes| bytes > 0), "the space is read");

        let closed = Cancellation::new();
        closed.cancel();
        let read = glib::MainContext::default().block_on(size_on_disk(&uri, &closed));

        assert_eq!(read, None, "nothing is read once the dialog closed");
    }
}
