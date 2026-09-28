// SPDX-License-Identifier: AGPL-3.0-only
//! What the windows of one application share: the skin, the settings file,
//! the standard folders, the network locations visited this session and
//! the way files are opened.
//!
//! The Python app kept these on its `Gtk.Application` (`settings_store`,
//! `visited_network`, `launch_default` in `desktop/winspace.py`) and
//! broadcast `environmentChanged` to every window. Here an [`AppContext`]
//! emits `places-changed`, which every window connects to, when a pin, a
//! saved share, a visited server, a standard folder ([`known_folders`]) or
//! a preference changes, and `layout-reset` when Settings restores the
//! default pane widths.

mod known_folders;

use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::Entry;
use ox_core::places::FolderLocations;
use ox_core::settings::{Bookmark, PreferencesUpdate, RecentEntry, Settings, SettingsData, SettingsError};

use crate::places;
use crate::settings_store::{Change, Reply, SettingsStore};
use crate::theme::Skin;

/// Emitted when the sidebar or the landing pages may list something else.
const PLACES_CHANGED: &str = "places-changed";

/// Emitted when every window returns its sidebar and columns to their
/// default widths ("Reset sidebar and column widths" in Settings).
const LAYOUT_RESET: &str = "layout-reset";

mod imp {
    use std::cell::{Cell, OnceCell, RefCell};
    use std::rc::Rc;
    use std::sync::OnceLock;

    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;
    use gtk::{gio, glib};
    use ox_core::places::Place;
    use ox_core::settings::Bookmark;

    use super::{LAYOUT_RESET, PLACES_CHANGED};
    use crate::settings_store::SettingsStore;
    use crate::theme::Skin;

    /// Private state of [`super::AppContext`].
    #[derive(Debug, Default)]
    pub(crate) struct AppContext {
        /// The skin every window draws with.
        pub(super) skin: OnceCell<Skin>,
        /// The shared settings file and its queue of changes.
        pub(super) settings: OnceCell<Rc<SettingsStore>>,
        /// SMB servers and shares browsed this session, oldest first.
        pub(super) visited_network: RefCell<Vec<Bookmark>>,
        /// Quick access rows of the standard folders, as last read from
        /// `user-dirs.dirs`.
        pub(super) known_folders: RefCell<Vec<Place>>,
        /// Reports changes of `user-dirs.dirs`, so the standard folders
        /// are read again; `None` where the file cannot be watched.
        pub(super) user_dirs_monitor: RefCell<Option<gio::FileMonitor>>,
        /// The number of the latest reading of `user-dirs.dirs` started; a
        /// reading that finishes after a newer one started is dropped.
        pub(super) latest_folder_reading: Cell<u64>,
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
    fn with_folder_locations(skin: Skin, settings: Settings, folder_locations: FolderLocations) -> Self {
        let context: Self = glib::Object::new();
        let imp = context.imp();
        imp.skin.set(skin).expect("a new AppContext has no skin yet");
        imp.settings
            .set(SettingsStore::new(settings))
            .expect("a new AppContext has no settings yet");
        context.watch_known_folders(folder_locations);
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

    /// The SMB servers and shares browsed this session, oldest first.
    pub(crate) fn visited_network(&self) -> Vec<Bookmark> {
        self.imp().visited_network.borrow().clone()
    }

    /// Records a browsed SMB location under Network for this session, as
    /// `remember_network` in winspace.py: the server, or the share the
    /// location is on. Browsing never saves a bookmark.
    pub(crate) fn remember_network(&self, uri: &str) {
        let Some(root) = places::visited_root(uri) else {
            return;
        };
        let visited = &self.imp().visited_network;
        let is_known = visited.borrow().iter().any(|known| known.uri == root.uri);
        if is_known {
            return;
        }
        visited.borrow_mut().push(root);
        self.notify_places_changed();
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

    /// Opens `entry` in its default application and records it among the
    /// recently opened files, as `launch_default` in winspace.py does.
    /// `on_error` hears GIO's reason when it could not be opened.
    pub(crate) fn open_file(
        &self,
        entry: &Entry,
        window: &gtk::Window,
        on_error: impl FnOnce(glib::Error) + 'static,
    ) {
        let recent = recent_entry(entry);
        let uri = entry.navigation_uri().to_owned();
        // Test safety: tests record the file instead of starting a real
        // application on the developer's desktop.
        #[cfg(test)]
        if let Some(launches) = self.imp().recorded_launches.borrow_mut().as_mut() {
            launches.push(uri);
            return;
        }
        let launch_context = WidgetExt::display(window).app_launch_context();
        let context = self.downgrade();
        glib::spawn_future_local(async move {
            let launched = gio::AppInfo::launch_default_for_uri_future(&uri, Some(&launch_context)).await;
            match (launched, context.upgrade()) {
                (Err(error), _) => on_error(error),
                (Ok(()), Some(context)) => context.remember_open(recent),
                (Ok(()), None) => {}
            }
        });
    }

    /// Records `recent` at the top of the recently opened files.
    fn remember_open(&self, recent: RecentEntry) {
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
fn recent_entry(entry: &Entry) -> RecentEntry {
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
