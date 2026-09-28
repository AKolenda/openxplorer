// SPDX-License-Identifier: AGPL-3.0-only
//! What the windows of one application share: the skin, the settings file,
//! the network locations visited this session and the way files are opened.
//!
//! The Python app kept these on its `Gtk.Application` (`settings_store`,
//! `visited_network`, `launch_default`) and broadcast `environmentChanged`
//! to every window. Here an [`AppContext`] emits `places-changed`, which
//! every window connects to, when a pin, a saved share, a visited server or
//! a preference changes.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};
use ox_core::entry::Entry;
use ox_core::settings::{Bookmark, RecentEntry, Settings, SettingsData, SettingsError};

use crate::places;
use crate::settings_store::{Change, Reply, SettingsStore};
use crate::theme::Skin;

/// Emitted when the sidebar or the landing pages may list something else.
const PLACES_CHANGED: &str = "places-changed";

mod imp {
    use std::cell::{OnceCell, RefCell};
    use std::rc::Rc;
    use std::sync::OnceLock;

    use gtk::glib;
    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;
    use ox_core::settings::Bookmark;

    use super::PLACES_CHANGED;
    use crate::settings_store::SettingsStore;
    use crate::theme::Skin;

    /// Private state of [`super::AppContext`].
    #[derive(Debug, Default)]
    pub struct AppContext {
        /// The skin every window draws with.
        pub(super) skin: OnceCell<Rc<Skin>>,
        /// The shared settings file and its queue of changes.
        pub(super) settings: OnceCell<Rc<SettingsStore>>,
        /// SMB servers and shares browsed this session, oldest first.
        pub(super) visited_network: RefCell<Vec<Bookmark>>,
        /// In tests, the files that would have been opened; tests must
        /// never start real applications.
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
            SIGNALS.get_or_init(|| vec![Signal::builder(PLACES_CHANGED).build()])
        }
    }
}

glib::wrapper! {
    /// The state every window of one application shares.
    pub struct AppContext(ObjectSubclass<imp::AppContext>);
}

impl AppContext {
    /// Shares `skin` and the settings read at startup between windows.
    ///
    /// # Panics
    ///
    /// Never: a new object has no skin or settings yet.
    pub fn new(skin: Rc<Skin>, settings: Settings) -> Self {
        let context: Self = glib::Object::new();
        let imp = context.imp();
        let is_new = imp.skin.set(skin).is_ok() && imp.settings.set(SettingsStore::new(settings)).is_ok();
        assert!(is_new, "a new AppContext has no skin or settings yet");
        context
    }

    /// The display skin shared by every window.
    pub fn skin(&self) -> &Rc<Skin> {
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
            let changed = result.is_ok();
            reply(result);
            if let (true, Some(context)) = (changed, context.upgrade()) {
                context.notify_places_changed();
            }
        });
        self.settings().change(change, after);
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
    pub(crate) fn notify_places_changed(&self) {
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
    /// `on_error` hears why it could not be opened.
    pub(crate) fn open_file(
        &self,
        entry: &Entry,
        window: &gtk::Window,
        on_error: impl FnOnce(String) + 'static,
    ) {
        let recent = recent_entry(entry);
        let uri = entry.navigation_uri().to_owned();
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
                (Err(error), _) => on_error(error.message().to_owned()),
                (Ok(()), Some(context)) => context.remember_open(recent),
                (Ok(()), None) => {}
            }
        });
    }

    fn remember_open(&self, recent: RecentEntry) {
        let change: Change = Box::new(move |settings| settings.remember_open(&recent));
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

/// The recent-files record of an opened entry.
fn recent_entry(entry: &Entry) -> RecentEntry {
    RecentEntry {
        uri: entry.uri.clone(),
        name: entry.name.clone(),
        type_name: entry.type_label.clone(),
        is_dir: entry.is_dir,
        size: entry.size.unwrap_or(0),
        modified: entry.modified,
    }
}
