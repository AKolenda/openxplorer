// SPDX-License-Identifier: AGPL-3.0-only
//! What the windows of one application share: the skin, the settings file,
//! the standard folders, the network services (the keyring, sign-outs and
//! the network locations visited this session) and the way files are
//! opened.
//!
//! The Python app kept these on its `Gtk.Application` (`settings_store`,
//! `visited_network`, `launch_default` in `desktop/winspace.py`) and
//! broadcast `environmentChanged` to every window. Here an [`AppContext`]
//! emits `places-changed`, which every window connects to, when a pin, a
//! saved share, a visited server, a standard folder ([`known_folders`]) or
//! a preference changes, and `layout-reset` when Settings restores the
//! default pane widths. The network services are [`network_places`]'s.
//! Every window shares the previous-versions service
//! ([`previous_versions`]), whose protection the file operations run with,
//! and the undo journal of the file operations ([`file_operations`]). It
//! also holds the search cache the windows share ([`search_cache`]), and
//! the application's [`Updates`] and [`DesktopIntegration`], so every
//! window shows the same update and integration state.

mod default_open;
mod external_open;
mod file_operations;
mod known_folders;
mod network_places;
mod previous_versions;
mod saved_searches;
mod search_cache;

pub(crate) use default_open::{add_to_desktop_history, FOLDER_CONTENT_TYPE};

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::entry::Entry;
use ox_core::folder_locations::FolderRelocation;
use ox_core::places::FolderLocations;
use ox_core::settings::{PreferencesUpdate, RecentEntry, Settings, SettingsData, SettingsError};
use ox_core::versions::PreviousVersions;

use crate::integration::DesktopIntegration;
use crate::settings_store::{Change, Reply, SettingsStore};
use crate::theme::Skin;
use crate::update::Updates;

/// Emitted when the sidebar or the landing pages may list something else.
const PLACES_CHANGED: &str = "places-changed";

/// Emitted when every window returns its sidebar and columns to their
/// default widths ("Reset sidebar and column widths" in Settings).
const LAYOUT_RESET: &str = "layout-reset";

/// Emitted when Undo or Redo would now do something else, so every window
/// relabels the commands.
const JOURNAL_CHANGED: &str = "journal-changed";

/// Emitted with a server's host and whether to clear its cached file
/// names, once Sign out of server finished (NET-022).
const SERVER_SIGNED_OUT: &str = "server-signed-out";

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::rc::Rc;
    use std::sync::{Arc, OnceLock};

    use gtk::glib::subclass::Signal;
    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gio, glib};
    use ox_core::folder_locations::FolderRelocation;
    use ox_core::ops::UndoJournal;
    use ox_core::places::Place;
    use ox_core::versions::PreviousVersions;

    use super::{JOURNAL_CHANGED, LAYOUT_RESET, PLACES_CHANGED, SERVER_SIGNED_OUT};
    use crate::integration::DesktopIntegration;
    use crate::network::NetworkServices;
    use crate::search::SearchCache;
    use crate::settings_store::SettingsStore;
    use crate::theme::Skin;
    use crate::update::Updates;

    /// Private state of [`super::AppContext`].
    #[derive(Debug, Default)]
    pub(crate) struct AppContext {
        /// The skin every window draws with.
        pub(super) skin: OnceCell<Skin>,
        /// The shared settings file and its queue of changes.
        pub(super) settings: OnceCell<Rc<SettingsStore>>,
        /// The keyring, the servers being signed out, the network
        /// locations browsed this session and the kernel's SMB mounts.
        pub(super) network: NetworkServices,
        /// Quick access rows of the standard folders, as last read from
        /// `user-dirs.dirs`.
        pub(super) known_folders: RefCell<Vec<Place>>,
        /// Reports changes of `user-dirs.dirs`, so the standard folders
        /// are read again; `None` where the file cannot be watched.
        pub(super) user_dirs_monitor: RefCell<Option<gio::FileMonitor>>,
        /// The number of the latest reading of `user-dirs.dirs` started; a
        /// reading that finishes after a newer one started is dropped.
        pub(super) latest_folder_reading: Cell<u64>,
        /// What Undo and Redo can do, shared by every window as Dolphin's
        /// undo manager is.
        pub(super) undo_journal: RefCell<UndoJournal>,
        /// The previous-versions service, whose read-only rule every file
        /// operation's worker asks, hence shared across threads.
        pub(super) previous_versions: OnceCell<Arc<PreviousVersions>>,
        /// The search cache and its index service.
        pub(super) search_cache: SearchCache,
        /// The searches saved to the sidebar, as last read (SRCH-038).
        pub(super) saved_searches: RefCell<Vec<ox_core::search::SavedSearch>>,
        /// The number of the reading the saved searches come from; an
        /// older one that arrives later is dropped.
        pub(super) saved_searches_reading: Cell<u64>,
        /// Moves the standard folders (the Properties Location tab); a
        /// test replaces it with one over its own folders.
        pub(super) folder_relocation: RefCell<Option<Arc<FolderRelocation>>>,
        /// The application's updates, made on first use.
        pub(super) updates: OnceCell<Updates>,
        /// The desktop integration, made on first use.
        pub(super) desktop_integration: OnceCell<DesktopIntegration>,
        /// In tests, the files that would have been opened.
        #[cfg(test)]
        pub(super) recorded_launches: RefCell<Option<Vec<String>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AppContext {
        const NAME: &'static str = "OxAppContext";
        type Type = super::AppContext;
    }

    impl ObjectImpl for AppContext {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| {
                vec![
                    Signal::builder(PLACES_CHANGED).build(),
                    Signal::builder(LAYOUT_RESET).build(),
                    Signal::builder(JOURNAL_CHANGED).build(),
                    Signal::builder(SERVER_SIGNED_OUT)
                        .param_types([String::static_type(), bool::static_type()])
                        .build(),
                ]
            })
        }
    }
}

glib::wrapper! {
    /// The state every window of one application shares.
    pub(crate) struct AppContext(ObjectSubclass<imp::AppContext>);
}

impl AppContext {
    /// Shares `skin` and the settings read at startup between windows, and
    /// the user's standard folders.
    pub(crate) fn new(skin: Skin, settings: Settings) -> Self {
        Self::with_folder_locations(skin, settings, FolderLocations::from_environment())
    }

    /// [`Self::new`] with the standard folders of `folder_locations`, so
    /// tests can move them without touching the user's own
    /// `user-dirs.dirs`.
    ///
    /// # Panics
    ///
    /// Never: a new object has no skin or settings yet.
    pub(crate) fn with_folder_locations(
        skin: Skin,
        settings: Settings,
        folder_locations: FolderLocations,
    ) -> Self {
        let context: Self = glib::Object::new();
        let imp = context.imp();
        imp.skin.set(skin).expect("a new AppContext has no skin yet");
        let versions = PreviousVersions::new(settings.directory());
        imp.previous_versions
            .set(Arc::new(versions))
            .expect("a new AppContext has no previous-versions service yet");
        let relocation = FolderRelocation::new(folder_locations.clone(), settings.directory().to_owned());
        imp.folder_relocation.replace(Some(Arc::new(relocation)));
        imp.settings
            .set(SettingsStore::new(settings))
            .expect("a new AppContext has no settings yet");
        context.watch_known_folders(folder_locations);
        context.read_saved_searches();
        context
    }

    /// The display skin shared by every window.
    pub(crate) fn skin(&self) -> &Skin {
        self.imp()
            .skin
            .get()
            .expect("AppContext::new is the only constructor and sets the skin")
    }

    fn settings(&self) -> &Rc<SettingsStore> {
        self.imp()
            .settings
            .get()
            .expect("AppContext::new is the only constructor and sets the settings")
    }

    /// The settings as last read or changed.
    pub(crate) fn settings_data(&self) -> SettingsData {
        self.settings().data()
    }

    /// The settings folder (`~/.config/winspace`).
    pub(crate) fn settings_directory(&self) -> PathBuf {
        self.settings().directory()
    }

    /// Moves the standard folders, for the Properties Location tab.
    ///
    /// # Panics
    ///
    /// Never: the constructor sets it.
    pub(crate) fn folder_relocation(&self) -> Arc<FolderRelocation> {
        let relocation = self.imp().folder_relocation.borrow();
        Arc::clone(
            relocation
                .as_ref()
                .expect("the constructor sets the folder relocation"),
        )
    }

    /// Moves the standard folders with `relocation` from now on.
    #[cfg(test)]
    pub(crate) fn use_folder_relocation(&self, relocation: FolderRelocation) {
        self.imp().folder_relocation.replace(Some(Arc::new(relocation)));
    }

    /// The application's updates, shared by every window.
    pub(crate) fn updates(&self) -> &Updates {
        self.imp().updates.get_or_init(Updates::for_this_build)
    }

    /// Uses `updates` instead of this build's, for tests with a simulated
    /// GitHub and package manager.
    ///
    /// # Panics
    ///
    /// When the updates were used already.
    #[cfg(test)]
    pub(crate) fn use_updates(&self, updates: Updates) {
        self.imp()
            .updates
            .set(updates)
            .expect("the test sets the updates before using them");
    }

    /// The desktop integration, shared by every window.
    pub(crate) fn desktop_integration(&self) -> &DesktopIntegration {
        self.imp()
            .desktop_integration
            .get_or_init(|| DesktopIntegration::new(&self.settings_directory()))
    }

    /// Uses `integration` instead of the desktop's, for tests that must
    /// not change the associations of the session they run in.
    ///
    /// # Panics
    ///
    /// When the integration was used already.
    #[cfg(test)]
    pub(crate) fn use_desktop_integration(&self, integration: DesktopIntegration) {
        self.imp()
            .desktop_integration
            .set(integration)
            .expect("the test sets the integration before using it");
    }

    /// Why the settings fell back to defaults at startup, if they did.
    pub(crate) fn settings_warning(&self) -> Option<String> {
        self.settings().warning()
    }

    /// Re-reads the settings file off the main thread and tells every
    /// window when another process changed it.
    pub(crate) fn reload_settings(&self) {
        let context = self.downgrade();
        self.settings().reload(move |changed| {
            let Some(context) = context.upgrade() else {
                return;
            };
            if changed {
                context.notify_places_changed();
            }
        });
    }

    /// Queues a settings change off the main thread. On success every
    /// window hears `places-changed`; `reply` hears the outcome either way.
    pub(crate) fn change_settings(
        &self,
        change: Change,
        reply: impl FnOnce(Result<(), SettingsError>) + 'static,
    ) {
        let context = self.downgrade();
        let after: Reply = Box::new(move |result| {
            let succeeded = result.is_ok();
            reply(result);
            if !succeeded {
                return;
            }
            if let Some(context) = context.upgrade() {
                context.notify_places_changed();
            }
        });
        self.settings().change(change, after);
    }

    /// Saves every valid value of `update` off the main thread, as
    /// `update_preferences` in `desktop/core.py` does; `reply` hears the
    /// outcome.
    pub(crate) fn update_preferences(
        &self,
        update: PreferencesUpdate,
        reply: impl FnOnce(Result<(), SettingsError>) + 'static,
    ) {
        let change: Change =
            Box::new(move |settings: &mut Settings| settings.update_preferences(&update).map(|_| ()));
        self.change_settings(change, reply);
    }

    /// Tells every window to return its sidebar and columns to their
    /// default widths.
    pub(crate) fn announce_layout_reset(&self) {
        self.emit_by_name::<()>(LAYOUT_RESET, &[]);
    }

    /// Calls `callback` whenever the layout is reset in any window.
    pub(crate) fn connect_layout_reset(&self, callback: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(LAYOUT_RESET, false, move |_| {
            callback();
            None
        })
    }

    /// Tells every window to redraw its sidebar and landing page.
    fn notify_places_changed(&self) {
        self.emit_by_name::<()>(PLACES_CHANGED, &[]);
    }

    /// Calls `callback` whenever the places may have changed.
    pub(crate) fn connect_places_changed(&self, callback: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(PLACES_CHANGED, false, move |_| {
            callback();
            None
        })
    }

    /// Records `recent` at the top of the recently opened files.
    pub(super) fn remember_open(&self, recent: RecentEntry) {
        let change: Change = Box::new(move |settings| settings.remember_open(recent));
        // Recording a recent file is best effort, as in the Python app: the
        // file already opened, and a busy settings lock must not say otherwise.
        self.change_settings(change, |_| {});
    }

    /// Records the files [`Self::open_file`] would open instead of opening
    /// them.
    #[cfg(test)]
    pub(crate) fn record_launches(&self) {
        self.imp().recorded_launches.replace(Some(Vec::new()));
    }

    /// The files recorded since [`Self::record_launches`].
    #[cfg(test)]
    pub(crate) fn recorded_launches(&self) -> Vec<String> {
        self.imp().recorded_launches.borrow().clone().unwrap_or_default()
    }
}

/// The recent-files record of an opened entry (`remember_open` in
/// `desktop/core.py` keeps these fields).
pub(super) fn recent_entry(entry: &Entry) -> RecentEntry {
    RecentEntry {
        uri: entry.uri.clone(),
        name: entry.name.clone(),
        type_label: entry.type_label.clone(),
        is_dir: entry.is_dir,
        size: entry.size.unwrap_or(0),
        // `settings.json` keeps 0 for an unknown time, as core.py does.
        modified: entry.modified.unwrap_or(0),
    }
}
