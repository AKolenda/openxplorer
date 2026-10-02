// SPDX-License-Identifier: AGPL-3.0-only
//! The desktop's recent-files privacy settings (SAFE-022).
//!
//! GNOME's Privacy settings say whether the desktop remembers the files a
//! user opens (`remember-recent-files`) and for how many days
//! (`recent-files-max-age`, -1 for ever). Nautilus honours them, and so
//! does `OpenXplorer`: while history is off, nothing is added to the
//! desktop's recently used list or to the recent files of
//! `settings.json`, which are forgotten, and the sidebar hides Recent
//! files; with a limit, the files opened longer ago are forgotten. "Clear
//! recent files" on the sidebar entry empties both lists at once.

use std::time::{SystemTime, UNIX_EPOCH};

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::settings::RecentEntry;

use super::{add_to_desktop_history, AppContext};
use crate::settings_store::Change;

/// The desktop's privacy settings.
const PRIVACY_SCHEMA: &str = "org.gnome.desktop.privacy";
/// Whether the desktop remembers opened files.
const REMEMBER_KEY: &str = "remember-recent-files";
/// For how many days, or -1 for ever.
const MAX_AGE_KEY: &str = "recent-files-max-age";

/// Seconds in a day.
const DAY: u64 = 24 * 60 * 60;

/// What the desktop allows the app to remember of the files opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecentPolicy {
    /// Whether opened files are remembered at all.
    pub(crate) remember: bool,
    /// For how many days; `None` for ever.
    pub(crate) max_age_days: Option<u64>,
}

impl RecentPolicy {
    /// Remember everything, for ever: a desktop without GNOME's privacy
    /// settings.
    const REMEMBER_ALL: Self = Self {
        remember: true,
        max_age_days: None,
    };

    /// What `settings`, GNOME's privacy settings, say.
    fn read(settings: &gio::Settings) -> Self {
        let max_age = settings.int(MAX_AGE_KEY);
        Self {
            // GTK takes an age of 0 as "remember nothing".
            remember: settings.boolean(REMEMBER_KEY) && max_age != 0,
            max_age_days: u64::try_from(max_age).ok(),
        }
    }

    /// The recent files to forget `now` (seconds since the Unix epoch).
    fn to_forget(self, now: u64) -> Forget {
        match (self.remember, self.max_age_days) {
            (false, _) => Forget::All,
            (true, None) => Forget::Nothing,
            (true, Some(days)) => Forget::OpenedBefore(now.saturating_sub(days.saturating_mul(DAY))),
        }
    }
}

/// Which recent files the privacy settings no longer allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Forget {
    /// None: history is kept for ever.
    Nothing,
    /// Every one: history is off.
    All,
    /// Those opened before this time, in seconds since the Unix epoch.
    OpenedBefore(u64),
}

/// GNOME's privacy settings, when the desktop has them with both keys.
fn privacy_settings() -> Option<gio::Settings> {
    let schema = gio::SettingsSchemaSource::default()?.lookup(PRIVACY_SCHEMA, true)?;
    let has_keys = schema.has_key(REMEMBER_KEY) && schema.has_key(MAX_AGE_KEY);
    has_keys.then(|| gio::Settings::new(PRIVACY_SCHEMA))
}

/// The stricter of GTK and the desktop privacy settings, also used by folder history.
pub(crate) fn desktop_recent_policy() -> RecentPolicy {
    let desktop = privacy_settings()
        .as_ref()
        .map_or(RecentPolicy::REMEMBER_ALL, RecentPolicy::read);
    with_gtk_policy(desktop)
}

fn with_gtk_policy(desktop: RecentPolicy) -> RecentPolicy {
    let Some(settings) = gtk::Settings::default() else {
        return desktop;
    };
    let gtk_age = settings.gtk_recent_files_max_age();
    let max_age_days = match (desktop.max_age_days, u64::try_from(gtk_age).ok()) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (left, right) => left.or(right),
    };
    RecentPolicy {
        remember: desktop.remember && settings.is_gtk_recent_files_enabled() && gtk_age != 0,
        max_age_days,
    }
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

impl AppContext {
    /// Follows the desktop's privacy settings: a change redraws the
    /// sidebar and forgets what may no longer be remembered.
    pub(super) fn follow_recent_privacy(&self) {
        if let Some(settings) = gtk::Settings::default() {
            for property in ["gtk-recent-files-enabled", "gtk-recent-files-max-age"] {
                settings.connect_notify_local(
                    Some(property),
                    glib::clone!(
                        #[weak(rename_to = context)]
                        self,
                        move |_, _| {
                            context.forget_old_recent_files();
                            context.notify_places_changed();
                        }
                    ),
                );
            }
        }
        let Some(settings) = privacy_settings() else {
            return;
        };
        settings.connect_changed(
            None,
            glib::clone!(
                #[weak(rename_to = context)]
                self,
                move |_, key| {
                    if [REMEMBER_KEY, MAX_AGE_KEY].contains(&key) {
                        context.forget_old_recent_files();
                        context.notify_places_changed();
                    }
                }
            ),
        );
        self.imp().privacy.replace(Some(settings));
        self.forget_old_recent_files();
    }

    /// What the desktop allows the app to remember of the files opened.
    pub(crate) fn recent_policy(&self) -> RecentPolicy {
        let privacy = self.imp().privacy.borrow();
        with_gtk_policy(
            privacy
                .as_ref()
                .map_or(RecentPolicy::REMEMBER_ALL, RecentPolicy::read),
        )
    }

    /// Records `recent`, just opened, in the recent files of the app and
    /// the desktop, as far as the privacy settings allow.
    pub(super) fn record_opened(&self, recent: RecentEntry, content_type: &str) {
        if !self.recent_policy().remember {
            self.forget_old_recent_files();
            return;
        }
        add_to_desktop_history(&recent.uri, content_type);
        let opened = RecentEntry {
            opened: Some(now()),
            ..recent
        };
        self.remember_open(opened);
        self.forget_old_recent_files();
    }

    /// Forgets the recent files the privacy settings no longer allow.
    fn forget_old_recent_files(&self) {
        match self.recent_policy().to_forget(now()) {
            Forget::Nothing => {}
            Forget::All => self.forget_recent(None),
            Forget::OpenedBefore(time) => self.forget_recent(Some(time)),
        }
    }

    /// Empties the recent files of the app and of the desktop ("Clear
    /// recent files"). Tests clear only a private data folder's list
    /// (native/tools/check.py), never the user's.
    pub(crate) fn clear_recent_files(&self) {
        self.forget_recent(None);
        #[cfg(test)]
        if !glib::user_data_dir().starts_with(std::env::temp_dir()) {
            return;
        }
        // Best effort, as recording is: an unreadable list has nothing to clear.
        let _ = gtk::RecentManager::default().purge_items();
    }

    /// Forgets the app's recent files opened before `opened_before`, or
    /// all of them for `None`.
    fn forget_recent(&self, opened_before: Option<u64>) {
        let recent = self.settings_data().recent;
        if recent.iter().all(|entry| entry.is_kept_by(opened_before)) {
            return;
        }
        let change: Change = Box::new(move |settings| settings.forget_recent(opened_before));
        self.change_settings(change, |_| {});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// parity: SAFE-022, SIDE-026
    #[gtk::test]
    fn both_recent_views_honor_desktop_privacy_even_when_gtk_history_is_enabled() {
        let Some(privacy) = privacy_settings() else {
            return;
        };
        let gtk = gtk::Settings::default().unwrap();
        let remembered = privacy.boolean(REMEMBER_KEY);
        let max_age = privacy.int(MAX_AGE_KEY);
        let gtk_enabled = gtk.is_gtk_recent_files_enabled();
        let gtk_age = gtk.gtk_recent_files_max_age();
        gtk.set_gtk_recent_files_enabled(true);
        gtk.set_gtk_recent_files_max_age(30);
        privacy.set_boolean(REMEMBER_KEY, true).unwrap();
        privacy.set_int(MAX_AGE_KEY, 30).unwrap();
        let fixture = tempfile::tempdir().unwrap();
        let uri = ox_core::location::file_uri(fixture.path());
        add_to_desktop_history(&uri, ox_core::integration::FOLDER_CONTENT_TYPE);
        assert!(gtk::RecentManager::default().has_item(&uri));
        assert!(crate::folder_view::recent_locations::recent_folder_uris().contains(&uri));
        privacy.set_boolean(REMEMBER_KEY, false).unwrap();
        assert!(!desktop_recent_policy().remember);
        assert!(crate::folder_view::recent_locations::recent_folder_uris().is_empty());
        privacy.set_boolean(REMEMBER_KEY, true).unwrap();
        privacy.set_int(MAX_AGE_KEY, 0).unwrap();
        assert!(!desktop_recent_policy().remember);
        assert!(crate::folder_view::recent_locations::recent_folder_uris().is_empty());
        privacy.set_int(MAX_AGE_KEY, 7).unwrap();
        assert_eq!(desktop_recent_policy().max_age_days, Some(7));
        gtk::RecentManager::default().remove_item(&uri).unwrap();
        privacy.set_boolean(REMEMBER_KEY, remembered).unwrap();
        privacy.set_int(MAX_AGE_KEY, max_age).unwrap();
        gtk.set_gtk_recent_files_enabled(gtk_enabled);
        gtk.set_gtk_recent_files_max_age(gtk_age);
    }
}
