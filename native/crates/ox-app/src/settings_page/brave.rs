// SPDX-License-Identifier: AGPL-3.0-only
//! Brave & downloads: Brave's download folder.
//!
//! Ports the "Brave & downloads" section of `appendV07Settings` in
//! `desktop/ui/app.js` (INT-020, SET-009). The sync itself
//! (`braveDialog`) is desktop integration, which the native preview does
//! not have yet, so its button waits for that milestone; the Python
//! section's advice stays as notes.

use super::group::SettingsGroup;
use super::pages::Category;
use super::parts;
use super::row::{Availability, ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use crate::icons::Icon;
use crate::window::{ButtonStyle, Milestone};

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
pub(super) fn build() -> SettingsSection {
    let category = Category::BraveAndDownloads;
    let brave = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    let group = SettingsGroup::new("Download folder");
    let row = SettingRow::new(USE_LINUX_DOWNLOADS);
    let sync = parts::button("Use Linux Downloads in Brave…", ButtonStyle::Accent);
    row.add_control(&sync, ControlName::OwnLabel);
    row.set_availability(Availability::Unported(Milestone::DesktopIntegration));
    group.add_row(&row);
    brave.append_group(&group);
    brave.append_text(&parts::note(Icon::Info, SYNC_NOTE));
    brave.append_text(&parts::note(Icon::Info, MANUAL_NOTE));
    brave
}
