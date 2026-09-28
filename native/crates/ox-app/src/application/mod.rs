// SPDX-License-Identifier: AGPL-3.0-only
//! Application lifetime: startup, launches, command-line options and the
//! application actions.
//!
//! Ports `activate_app` and `open_files` in `desktop/winspace.py` and the
//! window commands of `windowsMenu` in `desktop/ui/app.js`. Launching the
//! app again presents the open window instead of adding one; locations from
//! the command line or another app open in the active window (the first in
//! its current tab, the rest as tabs). Ctrl+N and `--new-window` open
//! another window.
//!
//! [`Application`] is a `GtkApplication` subclass: GTK calls its `startup`,
//! `activate`, `open` and `handle_local_options` methods, and it keeps the
//! [`AppState`] it creates at startup, which does the work ([`state`]).

mod state;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::snapshot::{self, SnapshotError, SnapshotRequest};

use state::{active_window, AppState};

/// The `--new-window` command-line option.
const NEW_WINDOW_OPTION: &str = "new-window";

/// The application action behind Ctrl+N and `--new-window`.
const NEW_WINDOW_ACTION: &str = "new-window";

/// How this process was started.
#[derive(Debug)]
enum Launch {
    /// A normal launch: the running instance, or this one when it is the
    /// first, shows a window.
    Interactive,
    /// The developer snapshot hook ([`crate::snapshot`]): one window, saved
    /// as a picture, in an instance of its own.
    Snapshot(SnapshotRequest),
}

/// Brings the window with `id` to the front (`focusWindow`), or says that
/// it closed while its menu was open.
fn focus_window(app: &gtk::Application, id: u32) {
    if let Some(window) = app.window_by_id(id) {
        window.present();
        return;
    }
    if let Some(window) = active_window(app) {
        window.notify("That window is no longer open.");
    }
}

/// "Quit OpenXplorer": closes every window through its close request, so
/// each one lets go of its tabs as a closed window does, and the
/// application ends with the last one.
fn close_every_window(app: &gtk::Application) {
    for window in app.windows() {
        window.close();
    }
}

mod imp {
    use std::cell::{Cell, OnceCell};
    use std::ops::ControlFlow;

    use gtk::prelude::*;
    use gtk::subclass::prelude::*;
    use gtk::{gio, glib};
    use ox_core::settings::Settings;

    use super::{AppState, Launch};

    /// Private state of [`super::Application`].
    #[derive(Debug, Default)]
    pub(super) struct Application {
        /// How the process was started; set before it runs.
        pub(super) launch: OnceCell<Launch>,
        /// Created at startup, once GTK has a display.
        pub(super) state: OnceCell<AppState>,
        /// Saving the snapshot failed, so the process exits with an error.
        pub(super) snapshot_failed: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "OxApplication";
        type Type = super::Application;
        type ParentType = gtk::Application;
    }

    impl ObjectImpl for Application {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().add_new_window_option();
            self.obj().install_actions();
        }
    }

    impl ApplicationImpl for Application {
        /// Creates the shared state once GTK has started.
        fn startup(&self) {
            self.parent_startup();
            let app = self.obj();
            if let Some(state) = AppState::new(app.upcast_ref(), Settings::open_default()) {
                self.state.set(state).expect("GTK starts an application once");
            }
        }

        /// Launching the app shows a window, or takes the snapshot.
        fn activate(&self) {
            let app = self.obj();
            let Some(state) = self.state.get() else {
                return;
            };
            match app.launch() {
                Launch::Interactive => state.activate(app.upcast_ref()),
                Launch::Snapshot(request) => app.take_snapshot(state, request),
            }
        }

        /// Another app or the command line asks to open `files`. A
        /// snapshot shows only the location it was asked for.
        fn open(&self, files: &[gio::File], _hint: &str) {
            let app = self.obj();
            let Some(state) = self.state.get() else {
                return;
            };
            if let Launch::Interactive = app.launch() {
                state.open(app.upcast_ref(), files);
            }
        }

        fn handle_local_options(&self, options: &glib::VariantDict) -> ControlFlow<glib::ExitCode> {
            self.obj().handle_new_window_option(options);
            ControlFlow::Continue(())
        }
    }

    impl GtkApplicationImpl for Application {}
}

glib::wrapper! {
    /// The OpenXplorer application: its windows and what they share.
    struct Application(ObjectSubclass<imp::Application>)
        @extends gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Application {
    /// The application for `launch`, under the preview's own ID. A
    /// snapshot runs as an instance of its own, so it never hands its
    /// window to a running preview.
    fn new(launch: Launch) -> Self {
        let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
        if matches!(launch, Launch::Snapshot(_)) {
            flags |= gio::ApplicationFlags::NON_UNIQUE;
        }
        let app: Self = glib::Object::builder()
            .property("application-id", crate::config::APP_ID)
            .property("flags", flags)
            .build();
        app.imp()
            .launch
            .set(launch)
            .expect("a new application has no launch yet");
        app
    }

    /// How the process was started.
    fn launch(&self) -> &Launch {
        self.imp()
            .launch
            .get()
            .expect("Application::new sets the launch before GTK runs it")
    }

    /// Adds `app.new-window`, `app.focus-window` and `app.quit`.
    fn install_actions(&self) {
        let new_window = gio::ActionEntry::builder(NEW_WINDOW_ACTION)
            .activate(|app: &Self, _, _| {
                if let Some(state) = app.imp().state.get() {
                    state.new_window(app.upcast_ref());
                }
            })
            .build();
        let focus_window = gio::ActionEntry::builder("focus-window")
            .parameter_type(Some(glib::VariantTy::UINT32))
            .activate(|app: &Self, _, target| {
                if let Some(id) = target.and_then(glib::Variant::get::<u32>) {
                    focus_window(app.upcast_ref(), id);
                }
            })
            .build();
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &Self, _, _| close_every_window(app.upcast_ref()))
            .build();
        self.add_action_entries([new_window, focus_window, quit]);
    }

    /// Accepts `--new-window` on the command line.
    fn add_new_window_option(&self) {
        self.add_main_option(
            NEW_WINDOW_OPTION,
            glib::Char::from(0),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            "Open a new window",
            None,
        );
    }

    /// `--new-window` asks the running instance (or this one, when it is
    /// the first) for another window; the launch then goes on as usual.
    /// When the application cannot register, the command line says why.
    fn handle_new_window_option(&self, options: &glib::VariantDict) {
        if !options.contains(NEW_WINDOW_OPTION) {
            return;
        }
        // Registering finds the running instance, which then runs the
        // action; unregistered, activating it would do nothing.
        if let Err(error) = self.register(None::<&gio::Cancellable>) {
            eprintln!("OpenXplorer: could not open a new window: {error}");
            return;
        }
        self.activate_action(NEW_WINDOW_ACTION, None);
    }

    /// Opens the window `request` describes, saves it once its first
    /// listing is drawn and quits, recording whether saving failed.
    fn take_snapshot(&self, state: &AppState, request: &SnapshotRequest) {
        let window = state.open_snapshot_window(self.upcast_ref(), request);
        let finish = glib::clone!(
            #[weak(rename_to = app)]
            self,
            move |outcome| app.finish_snapshot(outcome)
        );
        snapshot::save_when_listed(&window, request, finish);
    }

    /// Reports a snapshot that could not be saved, so the process exits
    /// with an error, and quits.
    fn finish_snapshot(&self, outcome: Result<(), SnapshotError>) {
        if let Err(error) = outcome {
            eprintln!("OpenXplorer snapshot: {error}");
            self.imp().snapshot_failed.set(true);
        }
        self.quit();
    }
}

/// Runs the preview under its own application ID, so installed
/// file-manager defaults and the production app's D-Bus name are untouched.
/// With `OPENXPLORER_SNAPSHOT` set it saves a picture of one window and
/// quits instead ([`crate::snapshot`]).
pub fn run() -> glib::ExitCode {
    let launch = match SnapshotRequest::from_environment() {
        Ok(Some(request)) => Launch::Snapshot(request),
        Ok(None) => Launch::Interactive,
        Err(error) => {
            eprintln!("OpenXplorer snapshot: {error}");
            return glib::ExitCode::FAILURE;
        }
    };
    let app = Application::new(launch);
    let status = app.run();
    if app.imp().snapshot_failed.get() {
        glib::ExitCode::FAILURE
    } else {
        status
    }
}
