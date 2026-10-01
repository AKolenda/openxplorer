// SPDX-License-Identifier: AGPL-3.0-only
//! Desktop integration as the running app offers it: the default file
//! manager and ZIP handler, "Show in folder" and its
//! `org.freedesktop.FileManager1` service, Brave's download folder, Open
//! with, Open in Terminal and the code-editor shortcuts.
//!
//! Ports the integration branches of `dispatch` and the `enable_reveal`,
//! `disable_reveal`, `reveal_status` and `handle_reveal` methods of
//! `desktop/winspace.py`, with the controls of `appendV07Settings`,
//! `braveDialog` and `openWithDialog` in `desktop/ui/app.js`. The rules
//! themselves are ox-core's ([`ox_core::integration`]); this module
//! runs them off the main thread and shows their results. Everything
//! stays opt-in: nothing changes a default, writes a session file or
//! touches Brave until the user asks (INT-010).
//!
//! [`DesktopIntegration`] is shared by every window of the application:
//! the `FileManager1` service belongs to the application, not to a window,
//! and keeps it running without a window while Show in folder is enabled
//! (INT-017).
//!
//! | Module | Responsibility |
//! |---|---|
//! | `mime_backend` | Where default handlers are read and set |
//! | `status` | [`IntegrationStatus`] and its texts |
//! | `file_manager_service` | The `FileManager1` service and the application hold |
//! | `changes` | Make default, Restore previous, the ZIP handler and Show in folder |
//! | `brave_dialog` | [`BraveDialog`], "Use this Downloads folder in Brave" |
//! | `open_with_dialog` | [`OpenWithDialog`], "Open with" |
//! | `applications` | The applications Open with lists |
//! | `terminal` | Open in Terminal |
//! | `editors` | The "Open in <editor>" shortcuts |
//! | `tools` | Compare Files and the preferred search tool |

mod applications;
mod brave_dialog;
mod changes;
mod custom_command;
mod editors;
mod file_manager_service;
mod mime_backend;
mod open_with_dialog;
mod status;
mod terminal;
#[cfg(test)]
mod tests;
mod tools;
mod type_associations;

use std::path::{Path, PathBuf};

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use ox_core::integration::{
    BraveIntegration, BravePaths, DefaultApps, RevealPaths, RevealRegistration, Sandbox, DESKTOP_PORTAL_NAME,
};

#[cfg(test)]
pub(crate) use applications::LaunchTarget;
pub(crate) use applications::{
    application_image, launch, menu_applications, prepare_launch, ApplicationChoice, DefaultChoice,
    OpenWithError, PreparedLaunch,
};
pub(crate) use brave_dialog::BraveDialog;
pub(crate) use changes::{IntegrationError, MakeDefaultChoice};
pub(crate) use editors::{editor_shortcuts_in_background, EditorShortcut};
pub(crate) use mime_backend::MimeBackend;
pub(crate) use open_with_dialog::{Launcher, OpenWithDialog, OpenWithSubject};
pub(crate) use status::{DefaultsReport, IntegrationStatus};
pub(crate) use terminal::open_terminal;
pub(crate) use tools::{installed_application, Tool};
pub(crate) use type_associations::{
    change_type, is_protected, other_applications, type_applications, TypeApplication, TypeChange,
};

/// Emitted when something the Settings status shows may have changed: the
/// `FileManager1` name was acquired, lost or released.
const CHANGED: &str = "changed";

/// The folders the integration reads and writes: the settings folder for
/// its records and backups, and the user's folders that hold the Show in
/// folder session files and Brave's profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntegrationFolders {
    /// The settings folder (`~/.config/winspace`).
    pub(crate) settings: PathBuf,
    /// `$XDG_CONFIG_HOME`: the autostart entry and Brave's profiles.
    pub(crate) config_home: PathBuf,
    /// `$XDG_DATA_HOME`: the D-Bus service file.
    pub(crate) data_home: PathBuf,
    /// The home folder, where sandboxed Brave installs live.
    pub(crate) home: PathBuf,
}

impl IntegrationFolders {
    /// The current user's folders, with the records in `settings`.
    pub(crate) fn for_user(settings: &Path) -> Self {
        Self {
            settings: settings.to_owned(),
            config_home: glib::user_config_dir(),
            data_home: glib::user_data_dir(),
            home: glib::home_dir(),
        }
    }

    /// Folders inside `root`, for tests, which must never write the
    /// user's own session files.
    #[cfg(test)]
    pub(crate) fn inside(root: &Path) -> Self {
        Self {
            settings: root.join("winspace"),
            config_home: root.join("config"),
            data_home: root.join("data"),
            home: root.to_owned(),
        }
    }
}

/// The ox-core services one user's integration is made of.
#[derive(Debug, Clone)]
struct Services {
    /// Where the app runs: on the host or inside Flatpak.
    sandbox: Sandbox,
    /// The default file manager and ZIP handler.
    defaults: DefaultApps<MimeBackend>,
    /// The Show in folder session files.
    reveal: RevealRegistration,
    /// Brave's download folder.
    brave: BraveIntegration,
    /// The settings folder, which keeps the records and backups.
    settings_directory: PathBuf,
    /// The bus name of the desktop portal that starts the Flatpak at login.
    background_portal: String,
}

mod imp {
    use std::cell::{OnceCell, RefCell};
    use std::rc::Rc;
    use std::sync::OnceLock;

    use gtk::glib::subclass::Signal;
    use gtk::subclass::prelude::*;
    use gtk::{gio, glib};
    use ox_core::integration::FileManagerBus;

    use super::file_manager_service::RequestHandler;
    use super::{EditorShortcut, Services, CHANGED};

    /// Private state of [`super::DesktopIntegration`].
    #[derive(Default)]
    pub(crate) struct DesktopIntegration {
        /// The ox-core services, set when it is made.
        pub(super) services: OnceCell<Services>,
        /// The `FileManager1` service while Show in folder is enabled.
        /// Shared with status reads that are still waiting for the bus.
        pub(super) file_manager: RefCell<Option<Rc<FileManagerBus>>>,
        /// Keeps the application running without a window while the
        /// service answers (`self.hold()` in `enable_reveal`).
        pub(super) service_hold: RefCell<Option<gio::ApplicationHoldGuard>>,
        /// The application, once it is attached.
        pub(super) application: glib::WeakRef<gio::Application>,
        /// Shows a `FileManager1` request in a window.
        pub(super) request_handler: OnceCell<Rc<RequestHandler>>,
        /// The installed code editors, read once on first use.
        pub(super) editors: OnceCell<Vec<EditorShortcut>>,
    }

    impl std::fmt::Debug for DesktopIntegration {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("DesktopIntegration")
                .field("services", &self.services)
                .field("file_manager", &self.file_manager)
                .field("service_hold", &self.service_hold)
                .finish_non_exhaustive()
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DesktopIntegration {
        const NAME: &'static str = "OxDesktopIntegration";
        type Type = super::DesktopIntegration;
    }

    impl ObjectImpl for DesktopIntegration {
        fn signals() -> &'static [Signal] {
            static SIGNALS: OnceLock<Vec<Signal>> = OnceLock::new();
            SIGNALS.get_or_init(|| vec![Signal::builder(CHANGED).build()])
        }
    }
}

glib::wrapper! {
    /// The desktop integration of the application, shared by its windows.
    pub(crate) struct DesktopIntegration(ObjectSubclass<imp::DesktopIntegration>);
}

impl DesktopIntegration {
    /// The integration of the current user, keeping its records in the
    /// settings folder `settings_directory` (`~/.config/winspace`).
    pub(crate) fn new(settings_directory: &Path) -> Self {
        let sandbox = Sandbox::detect();
        let folders = IntegrationFolders::for_user(settings_directory);
        Self::with_mime_backend(&folders, sandbox, MimeBackend::desktop(sandbox))
    }

    /// The integration in `folders`, with default handlers kept by
    /// `mime_backend`.
    pub(crate) fn with_mime_backend(
        folders: &IntegrationFolders,
        sandbox: Sandbox,
        mime_backend: MimeBackend,
    ) -> Self {
        Self::with_background_portal(folders, sandbox, mime_backend, DESKTOP_PORTAL_NAME)
    }

    /// The integration inside Flatpak, asking `background_portal`, the
    /// bus name of a stand-in portal, to start the app at login.
    #[cfg(test)]
    pub(crate) fn in_flatpak(
        folders: &IntegrationFolders,
        mime_backend: MimeBackend,
        background_portal: &str,
    ) -> Self {
        Self::with_background_portal(folders, Sandbox::Flatpak, mime_backend, background_portal)
    }

    fn with_background_portal(
        folders: &IntegrationFolders,
        sandbox: Sandbox,
        mime_backend: MimeBackend,
        background_portal: &str,
    ) -> Self {
        let reveal_paths = RevealPaths {
            settings: folders.settings.clone(),
            config_home: folders.config_home.clone(),
            data_home: folders.data_home.clone(),
        };
        let brave_paths = BravePaths {
            settings: folders.settings.clone(),
            home: folders.home.clone(),
            config_home: folders.config_home.clone(),
        };
        let services = Services {
            sandbox,
            defaults: DefaultApps::with_mime_defaults(&folders.settings, mime_backend),
            reveal: RevealRegistration::new(&reveal_paths, sandbox),
            brave: BraveIntegration::new(&brave_paths, sandbox),
            settings_directory: folders.settings.clone(),
            background_portal: background_portal.to_owned(),
        };
        let integration: Self = glib::Object::new();
        integration
            .imp()
            .services
            .set(services)
            .expect("a new DesktopIntegration has no services yet");
        integration
    }

    fn services(&self) -> &Services {
        self.imp()
            .services
            .get()
            .expect("DesktopIntegration::with_mime_backend sets the services")
    }

    /// Whether the app runs inside a Flatpak sandbox.
    pub(crate) fn sandbox(&self) -> Sandbox {
        self.services().sandbox
    }

    /// Brave's download-folder integration.
    pub(crate) fn brave(&self) -> BraveIntegration {
        self.services().brave.clone()
    }

    /// The settings folder, which keeps the integration's records.
    pub(crate) fn settings_directory(&self) -> &Path {
        &self.services().settings_directory
    }

    /// Calls `on_change` whenever the Settings status may have changed.
    pub(crate) fn connect_changed(&self, on_change: impl Fn() + 'static) -> glib::SignalHandlerId {
        self.connect_local(CHANGED, false, move |_| {
            on_change();
            None
        })
    }

    /// The installed code editors for "Open in <editor>", read off the
    /// main thread the first time they are needed.
    pub(crate) async fn editor_shortcuts(&self) -> Vec<EditorShortcut> {
        if let Some(editors) = self.imp().editors.get() {
            return editors.clone();
        }
        let editors = editor_shortcuts_in_background().await;
        self.imp().editors.get_or_init(|| editors).clone()
    }

    /// Offers `editors` as the installed code editors, for tests whose
    /// menus must not depend on the editors of the machine they run on.
    ///
    /// # Panics
    ///
    /// When the editors were read already.
    #[cfg(test)]
    pub(crate) fn use_editor_shortcuts(&self, editors: Vec<EditorShortcut>) {
        self.imp()
            .editors
            .set(editors)
            .expect("a test chooses the editors before they are read");
    }

    /// The code editors [`Self::editor_shortcuts`] has read, for menus
    /// built at once; empty until it finished the first time.
    pub(crate) fn known_editor_shortcuts(&self) -> Vec<EditorShortcut> {
        self.imp().editors.get().cloned().unwrap_or_default()
    }

    /// Tells the Settings pages to read the status again.
    fn notify_changed(&self) {
        self.emit_by_name::<()>(CHANGED, &[]);
    }
}
