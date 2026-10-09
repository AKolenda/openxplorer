// SPDX-License-Identifier: AGPL-3.0-only
//! Search: the folders to index and how the index keeps current.
//!
//! Ports the "Search cache" section of
//! `renderSettingsPage` in `v2.0.0:desktop/ui/app.js` (SET-006, SET-008,
//! SET-009), with the options of the settings mockup, including "Index
//! pinned folders automatically" (the owner's decision of 2026-09-28 that
//! anything pinned is indexed by default, SRCH-040). A status card says
//! how many folders and names instant search covers, with "Refresh all".
//! The folder list opens as a page of its own
//! ([`super::indexed_folders`]), with the limits of indexing. The note on
//! privacy is in the indexed folders row's ⓘ. Watching and the network interval are the
//! Python app's `autoIndex` and `networkInterval`, which the index service
//! follows at once.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::format;
use ox_core::search::{CacheStatus, PinIndexing};
use ox_core::settings::PreferencesUpdate;

use super::bindings::{Choice, PreferenceBinding};
use super::group::SettingsGroup;
use super::indexed_folders::{grouped_number, IndexCommand};
use super::pages::{Category, SettingsView, Subpage};
use super::parts;
use super::row::{ControlName, SettingRow};
use super::search::RowText;
use super::section::{PageKind, SettingsSection};
use super::status_card::{StatusCard, StatusText};
use super::{SettingsPage, SharedHandler, MESSAGE};
use crate::icons::Icon;

const FOLDERS_TO_INDEX: RowText = RowText {
    title: "Indexed folders",
    description: "Check a folder to index the names and paths of its files and subfolders. SMB \
                  folders work too.",
    keywords:
        "folders to index search cache add custom folder directory path local disk smb nas entire drive 2tb \
               indexing refresh clear",
};

const PINNED_FOLDERS: RowText = RowText {
    title: "Index pinned folders automatically",
    description: "Pinning a folder adds it to instant search. Unpinning removes it unless you \
                  added it yourself.",
    keywords: "quick access pin search cache",
};

const WATCH_FOLDERS: RowText = RowText {
    title: "Watch folders for changes",
    description: "Watch enabled local folders for changes while OpenXplorer is open.",
    keywords: "watch folders for live changes inotify automatic file updates index",
};

const NETWORK_CHECKS: RowText = RowText {
    title: "Check network and unwatched folders every",
    description: "SMB and unwatched folders are checked for changes this often.",
    keywords: "network / fallback checks network refresh interval smb polling nas seconds minute. SMB and unwatched folders \
               use incremental directory checks, not push notifications. Large trees take longer \
               than one interval.",
};

/// How often network and unwatched folders are checked, as the Python
/// select offers it.
const NETWORK_INTERVALS: [Choice<u32>; 3] = [
    Choice {
        value: 30,
        label: crate::i18n::message_id("30 seconds"),
    },
    Choice {
        value: 60,
        label: crate::i18n::message_id("1 minute"),
    },
    Choice {
        value: 300,
        label: crate::i18n::message_id("5 minutes"),
    },
];

/// The privacy note of the Python section.
const PRIVACY_NOTE: &str = crate::i18n::message_id(
    "Cached paths are stored locally and can be searched while a share is \
                            offline. Uncheck a root to stop caching and remove that root's names. \
                            Other overlapping roots may still contain them. Hidden folders and \
                            symbolic links are skipped.",
);

/// The status card's line while nothing is indexed.
const NOTHING_INDEXED: &str =
    crate::i18n::message_id("Search file names in the folders you choose, even while a share is offline.");

/// The Search page.
pub(super) fn build(page: &SettingsPage) -> SettingsSection {
    let category = Category::Search;
    let indexing = SettingsSection::new(category.title(), category.lead(), PageKind::Category);
    let card = status_card(page);
    indexing.append_card(&card);
    follow_cache_status(page, &card);
    indexing.append_group(&instant_search_group(page));
    indexing
}

/// Where instant search stands, with "Refresh all" (SRCH-023).
fn status_card(page: &SettingsPage) -> StatusCard {
    let refresh = parts::button_with_glyph(&ox_core::i18n::gettext("Refresh all"), Icon::ArrowClockwise);
    refresh.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.run_index_command(IndexCommand::RefreshAll)
    ));
    let status = StatusText {
        glyph: Icon::Search,
        title: "Instant search",
        text: ox_core::i18n::gettext_static(NOTHING_INDEXED),
        notice: None,
    };
    StatusCard::new(status, &[refresh.upcast()])
}

/// Keeps `card` showing the cache status as the search cache reads it.
fn follow_cache_status(page: &SettingsPage, card: &StatusCard) {
    let cache = page.context().search_cache().clone();
    show_cache_status(card, cache.status().as_ref());
    let handler = cache.connect_status_changed(glib::clone!(
        #[weak]
        card,
        #[weak]
        cache,
        move || show_cache_status(&card, cache.status().as_ref())
    ));
    page.imp().handlers.borrow_mut().push(SharedHandler {
        object: cache.upcast(),
        id: handler,
    });
}

/// Shows `status` on `card`: how many folders and names instant search
/// covers, and when it last finished a scan.
fn show_cache_status(card: &StatusCard, status: Option<&CacheStatus>) {
    let summary = status.map(CacheSummary::of).unwrap_or_default();
    card.set_title(&summary.title());
    card.set_text(&summary.text());
}

/// What the status card counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CacheSummary {
    /// Enabled indexed folders.
    folders: usize,
    /// Names they hold.
    names: u64,
    /// When the latest full scan finished.
    last_updated: Option<u64>,
}

impl CacheSummary {
    fn of(status: &CacheStatus) -> Self {
        let enabled = status.roots.iter().filter(|root| root.is_enabled());
        let folders = enabled.clone().count();
        let last_updated = enabled.filter_map(|root| root.updated).max();
        Self {
            folders,
            names: status.entry_count,
            last_updated,
        }
    }

    /// "Instant search is on for 5 folders", as the mockup's card says.
    fn title(self) -> String {
        match self.folders {
            0 => ox_core::i18n::gettext("Instant search is off"),
            1 => ox_core::i18n::gettext("Instant search is on for 1 folder"),
            folders => ox_core::i18n::format_message(
                "Instant search is on for {folders} folders",
                &[("folders", &folders.to_string())],
            ),
        }
    }

    /// "14,263 names indexed · last updated …".
    fn text(self) -> String {
        if self.folders == 0 {
            return ox_core::i18n::gettext_static(NOTHING_INDEXED).to_owned();
        }
        let names = ox_core::i18n::format_message(
            "{grouped_number} names indexed",
            &[("grouped_number", &grouped_number(self.names))],
        );
        let Some(updated) = self.last_updated else {
            return names;
        };
        let when = format::date_time_text(Some(updated));
        ox_core::i18n::format_message(
            "{names} · last updated {when}",
            &[("names", &names), ("when", &when)],
        )
    }
}

/// "Indexed folders", which opens the Indexed folders page, and how the
/// index keeps current.
fn instant_search_group(page: &SettingsPage) -> SettingsGroup {
    let group = SettingsGroup::new(&ox_core::i18n::gettext("Instant search"));
    let row = SettingRow::new(FOLDERS_TO_INDEX);
    row.add_details(ox_core::i18n::gettext_static(PRIVACY_NOTE));
    let manage = parts::page_button(&ox_core::i18n::gettext("Manage…"));
    manage.connect_clicked(glib::clone!(
        #[weak]
        page,
        move |_| page.show_view(SettingsView::Subpage(Subpage::IndexedFolders))
    ));
    row.add_control(&manage, ControlName::OwnLabel);
    group.add_row(&row);
    group.add_row(&pinned_folders_row(page));
    let watch = SettingRow::new(WATCH_FOLDERS);
    let auto_index = PreferenceBinding {
        read: |preferences| preferences.auto_index,
        write: |watch| PreferencesUpdate {
            auto_index: Some(watch),
            ..PreferencesUpdate::default()
        },
    };
    watch.add_control(&page.preference_switch(auto_index), ControlName::RowTitle);
    group.add_row(&watch);
    let network = SettingRow::new(NETWORK_CHECKS);
    let interval = PreferenceBinding {
        read: |preferences| preferences.network_interval,
        write: |seconds| PreferencesUpdate {
            network_interval: Some(seconds),
            ..PreferencesUpdate::default()
        },
    };
    let choice = page.preference_choice(&NETWORK_INTERVALS, interval);
    network.add_control(&choice, ControlName::RowTitle);
    group.add_row(&network);
    group
}

/// "Index pinned folders automatically" (SRCH-040), on by default. The
/// search cache keeps the switch, so the row shows the cache's value each
/// time Settings opens.
fn pinned_folders_row(page: &SettingsPage) -> SettingRow {
    let row = SettingRow::new(PINNED_FOLDERS);
    let switch = parts::switch();
    switch.set_active(true);
    switch.connect_active_notify(glib::clone!(
        #[weak]
        page,
        move |switch| {
            if page.is_user_change() {
                page.save_pin_indexing(switch.is_active());
            }
        }
    ));
    page.when_opened(glib::clone!(
        #[weak]
        page,
        #[weak]
        switch,
        move || page.show_pin_indexing(&switch)
    ));
    row.add_control(&switch, ControlName::RowTitle);
    row
}

impl SettingsPage {
    /// Shows the cache's "Index pinned folders automatically" on `switch`.
    fn show_pin_indexing(&self, switch: &gtk::Switch) {
        let cache = self.context().search_cache().clone();
        let page = self.downgrade();
        let switch = switch.downgrade();
        glib::spawn_future_local(async move {
            let Ok(indexing) = cache.pin_indexing().await else {
                return;
            };
            let (Some(page), Some(switch)) = (page.upgrade(), switch.upgrade()) else {
                return;
            };
            let is_on = indexing == PinIndexing::Automatic;
            page.while_showing_preferences(|| switch.set_active(is_on));
        });
    }

    /// Turns indexing of pinned folders on or off for the current pins.
    fn save_pin_indexing(&self, is_on: bool) {
        let indexing = if is_on {
            PinIndexing::Automatic
        } else {
            PinIndexing::Off
        };
        let pins = self.context().settings_data().pins;
        let cache = self.context().search_cache().clone();
        let page = self.downgrade();
        glib::spawn_future_local(async move {
            let outcome = cache.set_pin_indexing(pins, indexing).await;
            let (Err(error), Some(page)) = (outcome, page.upgrade()) else {
                return;
            };
            page.emit_by_name::<()>(MESSAGE, &[&error.to_string()]);
        });
    }
}

#[cfg(test)]
mod tests {
    use ox_core::settings::NETWORK_INTERVALS as ACCEPTED_INTERVALS;

    use super::*;

    /// parity: SET-016
    #[test]
    fn the_network_intervals_offered_are_the_ones_settings_accept() {
        let offered = NETWORK_INTERVALS.map(|choice| choice.value);
        assert_eq!(offered, ACCEPTED_INTERVALS);
    }

    /// parity: SRCH-022
    #[test]
    fn the_status_card_counts_folders_and_names() {
        let off = CacheSummary::default();
        let one = CacheSummary {
            folders: 1,
            names: 17,
            last_updated: None,
        };
        let many = CacheSummary {
            folders: 5,
            names: 14_263,
            last_updated: None,
        };

        assert_eq!(off.title(), "Instant search is off");
        assert_eq!(off.text(), NOTHING_INDEXED);
        assert_eq!(one.title(), "Instant search is on for 1 folder");
        assert_eq!(one.text(), "17 names indexed");
        assert_eq!(many.title(), "Instant search is on for 5 folders");
        assert_eq!(many.text(), "14,263 names indexed");
    }
}
