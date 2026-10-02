// SPDX-License-Identifier: AGPL-3.0-only
//! Application lifetime: startup, launches, command-line options and the
//! application actions.
//!
//! Ports `OpenXplorer` and `main` in `v2.0.0:desktop/winspace.py` and the window
//! commands of `windowsMenu` in `v2.0.0:desktop/ui/app.js`. The application is
//! unique: a later launch hands its command line and working directory to
//! the running instance and exits (INT-001). Launching again presents the
//! open window, or lists the windows when several are open; locations
//! from the command line or another app open in the active window (the
//! first in its current tab, the rest as tabs). Ctrl+N and `--new-window`
//! open another window. Before the application starts, the launch guard
//! handles `--version`, `--quit` and `--restart` and checks for an outdated
//! running instance ([`LaunchCheck`]).
//!
//! `Application` is a `GtkApplication` subclass: GTK calls its `startup`,
//! `activate`, `open` and `command_line` methods, and it keeps the
//! `AppState` it creates at startup, which does the work (`state.rs`).

mod clock_format;
mod command_line;
mod requests;
mod state;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gio, glib};

use crate::config::APP_ID;
use crate::snapshot::{self, SnapshotError, SnapshotRequest};
use crate::update::{LaunchCheck, RuntimeInfo};

use command_line::{CommandLineError, CommandOption, CommandRequest};
use state::{active_window, AppState};

/// The exit status of a command line the running instance refused, as
/// `argparse` errors had.
const INVALID_COMMAND_LINE: u8 = 2;

/// The renderer GTK draws with when `--software-rendering` is given: Cairo
/// on the processor, without the GPU (UPD-013).
const SOFTWARE_RENDERER: &str = "cairo";

/// The application actions (`app.*`), which menus, shortcuts and launcher
/// quick actions run by name; the enum keeps those names in one place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppAction {
    /// Opens another window (Ctrl+N, `newWindow` in app.js).
    NewWindow,
    /// Brings the window whose id is the `u32` target to the front
    /// (`focusWindow`).
    FocusWindow,
    /// Brings back the window whose id is the first value of the
    /// `(u, s, as)` target and opens there the folder (the second, empty
    /// when unknown) or reveals the items (the third): the Show button of
    /// a background operation's notification (INT-026).
    ShowDestination,
    /// Lists the open windows (`showWindows`, the launcher's "Open
    /// windows…").
    Windows,
    /// Opens Settings in the active window (`showSettings`).
    Settings,
    /// Closes every window, which ends the application.
    Quit,
}

impl AppAction {
    /// The name the application registers the action under.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            AppAction::NewWindow => "new-window",
            AppAction::FocusWindow => "focus-window",
            AppAction::ShowDestination => "show-destination",
            AppAction::Windows => "windows",
            AppAction::Settings => "settings",
            AppAction::Quit => "quit",
        }
    }

    /// The name widgets, menus and accelerators use: `app.` and the name.
    pub(crate) fn detailed_name(self) -> String {
        format!("app.{}", self.name())
    }
}

/// How this process was started.
#[derive(Debug)]
enum Launch {
    /// A normal launch: the running instance, or this one when it is the
    /// first, does what the command line asks.
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
        window.show_message(&ox_core::i18n::gettext("That window is no longer open."));
    }
}

mod imp {
    use std::cell::{Cell, OnceCell};

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
            super::command_line::add_options(&*self.obj());
            self.obj().install_actions();
        }
    }

    impl ApplicationImpl for Application {
        /// Names the application, then creates the shared state once GTK
        /// has started.
        fn startup(&self) {
            self.parent_startup();
            let app = self.obj();
            app.name_for_the_desktop();
            if let Some(state) = AppState::new(app.upcast_ref(), Settings::open_default()) {
                self.state.set(state).expect("GTK starts an application once");
            }
            if let Launch::Interactive = app.launch() {
                app.report_unfinished_operations();
            }
        }

        /// A launch without a command line, such as D-Bus activation from
        /// the dock: presents a window, or takes the snapshot.
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

        /// Another app asks to open `files`. A snapshot shows only the
        /// location it was asked for.
        fn open(&self, files: &[gio::File], _hint: &str) {
            let app = self.obj();
            let Some(state) = self.state.get() else {
                return;
            };
            if let Launch::Interactive = app.launch() {
                state.open(app.upcast_ref(), files);
            }
        }

        /// Stops the search index before the process ends, so another
        /// instance can take it over at once.
        fn shutdown(&self) {
            if let Some(state) = self.state.get() {
                state.shut_down();
            }
            self.parent_shutdown();
        }

        /// A launch's command line, this process's own or a later one's.
        fn command_line(&self, command_line: &gio::ApplicationCommandLine) -> glib::ExitCode {
            self.obj().run_command_line(command_line)
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
    /// The application for `launch`, under the build's ID
    /// ([`crate::config::APP_ID`]). A snapshot runs as an instance of its
    /// own, so it never hands its window to a running instance.
    fn new(launch: Launch) -> Self {
        let mut flags = gio::ApplicationFlags::HANDLES_OPEN | gio::ApplicationFlags::HANDLES_COMMAND_LINE;
        if matches!(launch, Launch::Snapshot(_)) {
            flags |= gio::ApplicationFlags::NON_UNIQUE;
        }
        let app: Self = glib::Object::builder()
            .property("application-id", APP_ID)
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

    /// Once the first window shows, tells the user what copies and moves
    /// that never finished, because the app stopped, left behind
    /// (OPS-038).
    fn report_unfinished_operations(&self) {
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = app)]
            self,
            async move {
                if let Some(window) = active_window(app.upcast_ref()) {
                    window.report_unfinished_operations().await;
                }
            }
        ));
    }

    /// Names the application as the desktop sees it (`startup` in
    /// winspace.py): the program name is the application ID, so the X11
    /// window class matches the launcher's `StartupWMClass`, and windows
    /// show the app's icon. Also publishes the build's identity for later
    /// launches ([`RuntimeInfo`]).
    fn name_for_the_desktop(&self) {
        name_the_process();
        RuntimeInfo::install(self);
    }

    /// Adds `app.new-window`, `app.focus-window`, `app.show-destination`,
    /// `app.windows`,
    /// `app.settings` and `app.quit`, which the launcher's quick actions
    /// and the windows menu run.
    fn install_actions(&self) {
        let new_window = gio::ActionEntry::builder(AppAction::NewWindow.name())
            .activate(|app: &Self, _, _| {
                if let Some(state) = app.imp().state.get() {
                    state.new_window(app.upcast_ref());
                }
            })
            .build();
        let focus_window = gio::ActionEntry::builder(AppAction::FocusWindow.name())
            .parameter_type(Some(glib::VariantTy::UINT32))
            .activate(|app: &Self, _, target| {
                if let Some(id) = target.and_then(glib::Variant::get::<u32>) {
                    focus_window(app.upcast_ref(), id);
                }
            })
            .build();
        let show_destination = gio::ActionEntry::builder(AppAction::ShowDestination.name())
            .parameter_type(Some(&<(u32, String, Vec<String>)>::static_variant_type()))
            .activate(|app: &Self, _, target| {
                if let Some(target) = target.and_then(glib::Variant::get) {
                    app.show_destination(target);
                }
            })
            .build();
        let windows = Self::state_entry(AppAction::Windows, AppState::show_windows);
        let settings = Self::state_entry(AppAction::Settings, AppState::open_settings);
        let quit = Self::state_entry(AppAction::Quit, |state, app| {
            state.quit_safely(app);
        });
        self.add_action_entries([
            new_window,
            focus_window,
            show_destination,
            windows,
            settings,
            quit,
        ]);
    }

    /// Opens a finished operation's destination in the window that ran
    /// it, or the active window once that one has closed.
    fn show_destination(&self, (id, folder, items): (u32, String, Vec<String>)) {
        let window = self
            .window_by_id(id)
            .and_downcast::<crate::window::BrowserWindow>();
        let Some(window) = window.or_else(|| active_window(self.upcast_ref())) else {
            return;
        };
        let folder = (!folder.is_empty()).then_some(folder.as_str());
        window.show_destination(folder, &items);
    }

    /// An action that runs `run` on the application state.
    fn state_entry(
        action: AppAction,
        run: impl Fn(&AppState, &gtk::Application) + 'static,
    ) -> gio::ActionEntry<Self> {
        gio::ActionEntry::builder(action.name())
            .activate(move |app: &Self, _, _| {
                if let Some(state) = app.imp().state.get() {
                    run(state, app.upcast_ref());
                }
            })
            .build()
    }

    /// Does what `command_line` asks (`command_line` in winspace.py), or
    /// says on its terminal why it cannot.
    fn run_command_line(&self, command_line: &gio::ApplicationCommandLine) -> glib::ExitCode {
        let Some(state) = self.imp().state.get() else {
            return glib::ExitCode::FAILURE;
        };
        if let Launch::Snapshot(request) = self.launch() {
            self.take_snapshot(state, request);
            return glib::ExitCode::SUCCESS;
        }
        let status = match CommandRequest::from_command_line(command_line) {
            Ok(request) => state.run_command(self.upcast_ref(), request),
            Err(error) => return refuse_command_line(&error),
        };
        self.explain_software_rendering(command_line);
        status
    }

    /// Tells the user, in the active window,
    /// that `--software-rendering` sent to a running instance that draws
    /// with the GPU takes a restart: GTK chooses one renderer per process.
    fn explain_software_rendering(&self, command_line: &gio::ApplicationCommandLine) {
        let asks = command_line
            .options_dict()
            .contains(CommandOption::SoftwareRendering.name());
        let renderer = std::env::var("GSK_RENDERER").ok();
        let Some(notice) = software_rendering_notice(asks, command_line.is_remote(), renderer.as_deref())
        else {
            return;
        };
        if let Some(window) = active_window(self.upcast_ref()) {
            window.show_message(notice);
        }
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

/// Refuses a command line the launch check let through, which only a
/// location that stopped being valid on the way can do, with status 2.
fn refuse_command_line(error: &CommandLineError) -> glib::ExitCode {
    glib::g_warning!(ox_core::LOG_DOMAIN, "Refused a command line: {error}");
    glib::ExitCode::from(INVALID_COMMAND_LINE)
}

/// Checks the command line in the launching process, which has the
/// terminal the user typed it in: an invalid one is reported there and
/// the launch exits with status 2, before it reaches the running
/// instance (`argparse` in `main`).
fn check_command_line(arguments: &[String]) -> Result<(), CommandLineError> {
    let (options, locations): (Vec<&String>, Vec<&String>) = arguments
        .iter()
        .skip(1)
        .partition(|argument| argument.starts_with("--"));
    let has_option = |option: CommandOption| {
        let name = format!("--{}", option.name());
        options.iter().any(|given| **given == name)
    };
    let resolved = locations
        .into_iter()
        .map(gio::File::for_commandline_arg)
        .map(|file| ox_core::location::normalise(&file.uri()))
        .collect::<Result<Vec<_>, _>>()?;
    CommandRequest::from_options(has_option, resolved).map(drop)
}

/// Asks GTK to draw without the GPU when the command line has
/// `--software-rendering`. It must run before GTK or any thread starts,
/// and applies only when this process becomes the running instance.
fn choose_renderer(arguments: &[String]) {
    let is_chosen = std::env::var_os("GSK_RENDERER").is_some();
    if let Some(renderer) = renderer_for(arguments, is_chosen) {
        std::env::set_var("GSK_RENDERER", renderer);
    }
}

/// The renderer `arguments` ask for: Cairo for `--software-rendering`,
/// unless the user already chose one (`is_chosen`), which is kept.
fn renderer_for(arguments: &[String], is_chosen: bool) -> Option<&'static str> {
    let option = format!("--{}", CommandOption::SoftwareRendering.name());
    let asks_software = arguments.iter().skip(1).any(|argument| *argument == option);
    (asks_software && !is_chosen).then_some(SOFTWARE_RENDERER)
}

/// What to tell a user whose `--software-rendering` reached a running
/// instance (`is_remote`) that draws with `renderer` rather than Cairo:
/// the option applies from a restart, which `--restart` offers.
fn software_rendering_notice(asks: bool, is_remote: bool, renderer: Option<&str>) -> Option<&'static str> {
    (asks && is_remote && renderer != Some(SOFTWARE_RENDERER)).then_some(
        "OpenXplorer is already running with hardware rendering. To use software rendering, \
         run: openxplorer --restart --software-rendering",
    )
}

/// Runs the app under the build's application ID (`APP_ID` in
/// `config.rs`); the preview's own ID leaves installed file-manager
/// defaults and the Python app's D-Bus name untouched. With
/// `OPENXPLORER_SNAPSHOT` set it saves a picture of one window and quits
/// instead (see `snapshot.rs`); otherwise the launch guard runs first.
/// The interface's translations are loaded before any text is made
/// (INT-031).
pub fn run() -> glib::ExitCode {
    ox_core::i18n::install();
    let arguments: Vec<String> = std::env::args().collect();
    let launch = match SnapshotRequest::from_environment() {
        Ok(Some(request)) => Launch::Snapshot(request),
        Ok(None) => Launch::Interactive,
        Err(error) => {
            eprintln!("OpenXplorer snapshot: {error}");
            return glib::ExitCode::FAILURE;
        }
    };
    choose_renderer(&arguments);
    if let Err(error) = check_command_line(&arguments) {
        eprintln!("{error}");
        return glib::ExitCode::from(INVALID_COMMAND_LINE);
    }
    let arguments = match launch {
        Launch::Snapshot(_) => match LaunchCheck::root_refusal() {
            Some(status) => return status,
            None => arguments,
        },
        Launch::Interactive => match LaunchCheck::run(arguments) {
            LaunchCheck::Continue(arguments) => arguments,
            LaunchCheck::Exit(status) => return status,
        },
    };
    let app = Application::new(launch);
    let status = app.run_with_args(&arguments);
    if app.imp().snapshot_failed.get() {
        glib::ExitCode::FAILURE
    } else {
        status
    }
}

/// Names the process `OpenXplorer` with the application ID as its program
/// name and window icon, so the X11 window class matches the launcher's
/// `StartupWMClass` (`startup` in winspace.py).
fn name_the_process() {
    glib::set_application_name("OpenXplorer");
    glib::set_prgname(Some(APP_ID));
    gtk::Window::set_default_icon_name(APP_ID);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn words(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    /// `--software-rendering` draws with Cairo for this launch only, and
    /// changes no desktop setting; a renderer the user chose is kept.
    ///
    /// parity: UPD-013, INT-006
    #[test]
    fn software_rendering_chooses_cairo_for_this_launch() {
        let software = words(&["openxplorer", "--new-window", "--software-rendering"]);
        assert_eq!(renderer_for(&software, false), Some("cairo"));
        assert_eq!(renderer_for(&software, true), None);
        assert_eq!(renderer_for(&words(&["openxplorer", "/tmp"]), false), None);
    }

    /// The dock groups the windows under the launcher: the program name
    /// is the launcher's `StartupWMClass`, and the icon its ID.
    ///
    /// parity: LOOK-011
    #[gtk::test]
    fn the_process_is_named_as_its_launcher() {
        name_the_process();
        assert_eq!(glib::application_name().as_deref(), Some("OpenXplorer"));
        assert_eq!(glib::prgname().as_deref(), Some(APP_ID));
        assert_eq!(gtk::Window::default_icon_name().as_deref(), Some(APP_ID));
        let launcher = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging/data")
            .join(format!("{APP_ID}.desktop"));
        let entry = std::fs::read_to_string(&launcher).expect("the build's launcher is packaged");
        assert!(entry
            .lines()
            .any(|line| line == format!("StartupWMClass={APP_ID}")));
    }

    /// `--new-window --software-rendering` sent to a running instance
    /// that draws with the GPU says how to restart into software
    /// rendering, instead of ignoring the option.
    ///
    /// parity: UPD-013
    #[test]
    fn software_rendering_for_a_running_instance_offers_a_restart() {
        let notice = software_rendering_notice(true, true, None).expect("a hardware instance explains");
        assert!(notice.contains("openxplorer --restart --software-rendering"));
        assert_eq!(software_rendering_notice(true, true, Some("cairo")), None);
        assert_eq!(software_rendering_notice(true, false, None), None);
        assert_eq!(software_rendering_notice(false, true, None), None);
    }
}
