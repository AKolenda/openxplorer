// SPDX-License-Identifier: AGPL-3.0-only
//! Brave & downloads: Brave's download folder.
//!
//! Ports the "Brave & downloads" section of `appendV07Settings` in
//! `desktop/ui/app.js` (INT-020, SET-009). "Use Linux Downloads in
//! Brave…" opens the Brave dialog (`braveDialog`) on the user's Downloads
//! folder, as `user-dirs.dirs` names it; the Python section's advice
//! stays as notes.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::places::{FolderLocations, KnownFolder};

use super::group::SettingsGroup;
use super::pages::Category;
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::SettingsPage;
use crate::icons::Icon;
use crate::integration::BraveDialog;
use crate::window::ButtonStyle;

const USE_LINUX_DOWNLOADS: RowText = RowText {
    title: "Use Linux Downloads in Brave",
    description: "Selected native Brave profiles save to your Linux Downloads folder.",
    keywords: "brave download location downloads sync save as browser profile. Use the Linux \
               Downloads location in selected native Brave profiles.",
};

/// What to do first and what the sync changes, from the Python section.
const SYNC_NOTE: &str = "Fully quit Brave first, including background processes. OpenXplorer \
                         backs up Preferences and changes only the download and Save as \
                         directories. This is a one-time sync, not a managed browser policy.";

/// What to do where the sync cannot reach, from the Python section.
const MANUAL_NOTE: &str = "Flatpak/Snap, custom profiles, or managed browsers: open \
                           brave://settings/downloads and choose the same mounted Linux directory \
                           manually. SMB bookmarks are not persistent download paths.";

/// The Brave & downloads page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::BraveAndDownloads;
    let brave = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    let group = SettingsGroup::new("Download folder");
    let row = SettingRow::new(USE_LINUX_DOWNLOADS);
    let sync = parts::button("Use Linux Downloads in Brave…", ButtonStyle::Accent);
    sync.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| open_brave_dialog(&page)
    ));
    row.add_control(&sync, ControlName::OwnLabel);
    group.add_row(&row);
    brave.append_group(&group);
    brave.append_text(&parts::note(Icon::Info, SYNC_NOTE));
    brave.append_text(&parts::note(Icon::Info, MANUAL_NOTE));
    brave
}

/// Opens the Brave dialog over the page's window, on the Downloads folder
/// read off the main thread.
fn open_brave_dialog(page: &SettingsPage) {
    let brave = page.context().desktop_integration().brave();
    glib::spawn_future_local(glib::clone!(
        #[weak]
        page,
        async move {
            let Ok(downloads) = gio::spawn_blocking(downloads_folder).await else {
                return;
            };
            let Some(window) = page.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let report = glib::clone!(
                #[weak]
                page,
                move |message: &str| page.report(message)
            );
            BraveDialog::present_for(&window, brave, &downloads, report);
        }
    ));
}

/// The user's Downloads folder, as `user-dirs.dirs` names it.
fn downloads_folder() -> String {
    let folders = FolderLocations::from_environment().read_paths();
    folders.path(KnownFolder::Downloads).display().to_string()
}
