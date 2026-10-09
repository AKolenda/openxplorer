// SPDX-License-Identifier: AGPL-3.0-only
//! Default apps ▸ Browsers: Brave's download folder.
//!
//! Ports the "Brave & downloads" section of `appendV07Settings` in
//! `v2.0.0:desktop/ui/app.js` (INT-020, SET-009). "Use Linux Downloads in
//! Brave…" opens the Brave dialog (`braveDialog`) on the user's Downloads
//! folder, as `user-dirs.dirs` names it; the Python section's advice is
//! in the row's ⓘ.

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::places::{FolderLocations, KnownFolder};

use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::SettingsPage;
use crate::integration::BraveDialog;
use crate::window::ButtonStyle;

const USE_LINUX_DOWNLOADS: RowText = RowText {
    title: "Brave saves to my Linux Downloads folder",
    description: "Selected native Brave profiles save to your Linux Downloads folder.",
    keywords: "use linux downloads in brave brave download location downloads sync save as browser profile. Use the Linux \
               Downloads location in selected native Brave profiles.",
};

/// What to do first and what the sync changes, from the Python section.
const SYNC_NOTE: &str = crate::i18n::message_id(
    "Fully quit Brave first, including background processes. OpenXplorer \
                         backs up Preferences and changes only the download and Save as \
                         directories. This is a one-time sync, not a managed browser policy.",
);

/// What to do where the sync cannot reach, from the Python section.
const MANUAL_NOTE: &str = crate::i18n::message_id(
    "Flatpak/Snap, custom profiles, or managed browsers: open \
                           brave://settings/downloads and choose the same mounted Linux directory \
                           manually. SMB bookmarks are not persistent download paths.",
);

/// The row whose button opens the Brave dialog, with the Python
/// section's advice in its ⓘ.
pub(super) fn downloads_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(USE_LINUX_DOWNLOADS);
    row.add_details(ox_core::i18n::gettext_static(SYNC_NOTE));
    row.add_details(ox_core::i18n::gettext_static(MANUAL_NOTE));
    let sync = parts::button(
        &ox_core::i18n::gettext("Use Linux Downloads in Brave…"),
        ButtonStyle::Accent,
    );
    sync.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |button| open_brave_dialog(&page, button)
    ));
    row.add_control(&sync, ControlName::OwnLabel);
    row
}

/// Opens the Brave dialog over the page's window, on the Downloads folder
/// read off the main thread. `button` stays off until the dialog shows,
/// so a second click while the profiles are read opens no second dialog.
fn open_brave_dialog(page: &SettingsPage, button: &gtk::Button) {
    let brave = page.context().desktop_integration().brave();
    button.set_sensitive(false);
    glib::spawn_future_local(glib::clone!(
        #[weak]
        page,
        #[weak]
        button,
        async move {
            let downloads = gio::spawn_blocking(downloads_folder).await;
            let window = page.root().and_downcast::<gtk::Window>();
            let (Ok(downloads), Some(window)) = (downloads, window) else {
                button.set_sensitive(true);
                return;
            };
            let report = glib::clone!(
                #[weak]
                page,
                move |message: &str| page.report(message)
            );
            let dialog = BraveDialog::present_for(&window, brave, &downloads, report);
            dialog.connect_map(glib::clone!(
                #[weak]
                button,
                move |_| button.set_sensitive(true)
            ));
        }
    ));
}

/// The user's Downloads folder, as `user-dirs.dirs` names it.
fn downloads_folder() -> String {
    let folders = FolderLocations::from_environment().read_paths();
    folders.path(KnownFolder::Downloads).display().to_string()
}
