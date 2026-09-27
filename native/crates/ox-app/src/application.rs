// SPDX-License-Identifier: AGPL-3.0-only
//! Application lifetime: windows, command-line locations and the shared
//! appearance.
//!
//! Ports `activate_app`, `open_files` and `create_window` in
//! `desktop/winspace.py`. Launching the app again presents the open window
//! instead of adding one; locations from the command line or another app
//! open in the active window (the first in its current tab, the rest as
//! tabs). Ctrl+N and `--new-window` open another window. New windows start
//! in the home folder.

use std::cell::{Cell, OnceCell};
use std::ops::ControlFlow;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::file_uri;
use ox_core::settings::Settings;

use crate::shared::AppContext;
use crate::snapshot::{self, SnapshotRequest};
use crate::theme::system::SystemScheme;
use crate::theme::{Skin, ThemePreference};
use crate::window::BrowserWindow;

/// The `--new-window` command-line option.
const NEW_WINDOW_OPTION: &str = "new-window";

/// What lives as long as the application: the shared state and the
/// desktop colour-scheme watch.
#[derive(Debug)]
struct Desktop {
    context: AppContext,
    /// Kept alive so the skin follows the desktop's light or dark scheme.
    _system_scheme: Rc<SystemScheme>,
}

impl Desktop {
    /// Installs the skin on the default display and starts watching the
    /// desktop's colour scheme. `None` without a display.
    fn new(app: &gtk::Application, settings: Settings) -> Option<Self> {
        let display = gtk::gdk::Display::default()?;
        let skin = Rc::new(Skin::install(&display));
        Some(Self::with_skin(app, skin, settings))
    }

    /// The application state around an installed `skin`.
    fn with_skin(app: &gtk::Application, skin: Rc<Skin>, settings: Settings) -> Self {
        let preferences = settings.data().preferences.clone();
        skin.set_preference(ThemePreference::parse(&preferences.theme));
        skin.set_text_size(preferences.text_size);
        let follower = Rc::downgrade(&skin);
        let system_scheme = SystemScheme::new(move |dark| {
            if let Some(skin) = follower.upgrade() {
                skin.set_system_dark(dark);
            }
        });
        skin.set_system_dark(system_scheme.is_dark());
        crate::window::install_accelerators(app);
        Self {
            context: AppContext::new(skin, settings),
            _system_scheme: system_scheme,
        }
    }

    /// Opens a window whose first tab shows `start`, or the home folder.
    fn open_window(&self, app: &gtk::Application, start: Option<&str>) -> BrowserWindow {
        let window = self.build_window(app, start);
        window.present();
        window
    }

    /// A window whose first tab shows `start`, or the home folder, not
    /// shown yet.
    fn build_window(&self, app: &gtk::Application, start: Option<&str>) -> BrowserWindow {
        let window = BrowserWindow::new(app, &self.context);
        let home = file_uri(&glib::home_dir());
        let start = start.unwrap_or(home.as_str());
        if let Err(error) = window.add_tab(start) {
            window.notify(error.message());
            // The home folder always resolves; a window never opens empty.
            let _ = window.add_tab(&home);
        }
        if let Some(warning) = self.context.settings_warning() {
            window.notify(&warning);
        }
        window
    }

    /// Opens the window `request` describes, saves it once its first
    /// listing is drawn and quits; `failed` records whether saving failed.
    fn take_snapshot(&self, app: &gtk::Application, request: &SnapshotRequest, failed: &Rc<Cell<bool>>) {
        if let Some(theme) = request.theme {
            self.context.skin().set_preference(theme);
        }
        let window = self.build_window(app, request.start.as_deref());
        if let Some(size) = request.size {
            window.set_default_size(size.width, size.height);
        }
        if let Some(view) = request.view {
            window.show_view(view);
        }
        window.present();
        let failed = Rc::clone(failed);
        snapshot::save_when_listed(
            &window,
            request,
            glib::clone!(
                #[weak]
                app,
                move |outcome| {
                    if let Err(error) = outcome {
                        eprintln!("OpenXplorer snapshot: {error}");
                        failed.set(true);
                    }
                    app.quit();
                }
            ),
        );
    }

    /// Presents the open window, or opens the first one.
    fn activate(&self, app: &gtk::Application) {
        match active_window(app) {
            Some(window) => window.present(),
            None => {
                self.open_window(app, None);
            }
        }
    }

    /// Opens `files` in the active window, as `open_files` does.
    fn open(&self, app: &gtk::Application, files: &[gio::File]) {
        let window = active_window(app).unwrap_or_else(|| self.open_window(app, None));
        let uris = files.iter().map(|file| file.uri().to_string()).collect();
        window.open_locations(uris);
        window.present();
    }

    /// Ctrl+N: another window at the active folder when it is a real
    /// folder, else at home (app.js `newWindow`).
    fn new_window(&self, app: &gtk::Application) {
        let current = active_window(app).and_then(|window| window.current_uri());
        let start = current.filter(|uri| uri.starts_with("file:") || uri.starts_with("smb:"));
        self.open_window(app, start.as_deref());
    }
}

/// The focused browser window, else the most recent one.
fn active_window(app: &gtk::Application) -> Option<BrowserWindow> {
    let focused = app.active_window().and_downcast::<BrowserWindow>();
    focused.or_else(|| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<BrowserWindow>().ok())
    })
}

fn install_app_actions(app: &gtk::Application, desktop: &Rc<OnceCell<Desktop>>) {
    let new_window = gio::ActionEntry::builder("new-window")
        .activate(glib::clone!(
            #[strong]
            desktop,
            move |app: &gtk::Application, _, _| {
                if let Some(desktop) = desktop.get() {
                    desktop.new_window(app);
                }
            }
        ))
        .build();
    let focus_window = gio::ActionEntry::builder("focus-window")
        .parameter_type(Some(glib::VariantTy::UINT32))
        .activate(|app: &gtk::Application, _, target| {
            if let Some(id) = target.and_then(glib::Variant::get::<u32>) {
                focus_window(app, id);
            }
        })
        .build();
    let quit = gio::ActionEntry::builder("quit")
        .activate(|app: &gtk::Application, _, _| close_every_window(app))
        .build();
    app.add_action_entries([new_window, focus_window, quit]);
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

/// Accepts `--new-window` on the command line.
fn add_new_window_option(app: &gtk::Application) {
    app.add_main_option(
        NEW_WINDOW_OPTION,
        glib::Char::from(0),
        glib::OptionFlags::NONE,
        glib::OptionArg::None,
        "Open a new window",
        None,
    );
    app.connect_handle_local_options(handle_local_options);
}

/// `--new-window` asks the running instance (or this one, when it is the
/// first) for another window, then continues like a normal launch.
fn handle_local_options(app: &gtk::Application, options: &glib::VariantDict) -> ControlFlow<glib::ExitCode> {
    if options.contains(NEW_WINDOW_OPTION) && app.register(None::<&gio::Cancellable>).is_ok() {
        app.activate_action("new-window", None);
    }
    ControlFlow::Continue(())
}

/// Runs the preview under its own application ID, so installed
/// file-manager defaults and the production app's D-Bus name are untouched.
/// With `OPENXPLORER_SNAPSHOT` set it saves a picture of one window and
/// quits instead ([`crate::snapshot`]).
pub fn run() -> glib::ExitCode {
    let snapshot = match SnapshotRequest::from_environment() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            eprintln!("OpenXplorer snapshot: {error}");
            return glib::ExitCode::FAILURE;
        }
    };
    let app = build_application(snapshot.is_some());
    add_new_window_option(&app);
    let desktop: Rc<OnceCell<Desktop>> = Rc::default();
    install_app_actions(&app, &desktop);
    connect_startup(&app, &desktop);
    let snapshot_failed = Rc::new(Cell::new(false));
    match snapshot {
        Some(request) => connect_snapshot(&app, &desktop, request, &snapshot_failed),
        None => connect_launches(&app, &desktop),
    }
    let status = app.run();
    if snapshot_failed.get() {
        glib::ExitCode::FAILURE
    } else {
        status
    }
}

/// The application. A snapshot runs as an instance of its own, so it never
/// hands its window to a running preview.
fn build_application(for_snapshot: bool) -> gtk::Application {
    let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
    if for_snapshot {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    gtk::Application::builder()
        .application_id(crate::config::APP_ID)
        .flags(flags)
        .build()
}

/// Creates the shared state once GTK has started.
///
/// # Panics
///
/// Never in practice: GTK emits `startup` once per application, and only
/// `startup` creates the shared state.
fn connect_startup(app: &gtk::Application, desktop: &Rc<OnceCell<Desktop>>) {
    app.connect_startup(glib::clone!(
        #[strong]
        desktop,
        move |app| {
            if let Some(started) = Desktop::new(app, Settings::open_default()) {
                desktop.set(started).expect("GTK starts an application once");
            }
        }
    ));
}

/// Launching the app, or asking it to open files, shows a window.
fn connect_launches(app: &gtk::Application, desktop: &Rc<OnceCell<Desktop>>) {
    app.connect_activate(glib::clone!(
        #[strong]
        desktop,
        move |app| {
            if let Some(desktop) = desktop.get() {
                desktop.activate(app);
            }
        }
    ));
    app.connect_open(glib::clone!(
        #[strong]
        desktop,
        move |app, files, _| {
            if let Some(desktop) = desktop.get() {
                desktop.open(app, files);
            }
        }
    ));
}

/// Launching the app takes the snapshot `request` asks for.
fn connect_snapshot(
    app: &gtk::Application,
    desktop: &Rc<OnceCell<Desktop>>,
    request: SnapshotRequest,
    failed: &Rc<Cell<bool>>,
) {
    app.connect_activate(glib::clone!(
        #[strong]
        desktop,
        #[strong]
        failed,
        move |app| {
            if let Some(desktop) = desktop.get() {
                desktop.take_snapshot(app, &request, &failed);
            }
        }
    ));
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;
    use crate::test_support::harness::{application, settle, skin, wait_until, Fixture};

    /// A desktop on the shared test application, with its own settings.
    fn desktop() -> (Desktop, TempDir) {
        let settings = tempfile::tempdir().expect("the test home has room for settings");
        let desktop = Desktop::with_skin(&application(), skin(), Settings::open(settings.path()));
        desktop.context.record_launches();
        (desktop, settings)
    }

    fn browser_windows() -> Vec<BrowserWindow> {
        let windows = application().windows();
        let browsers = windows
            .into_iter()
            .filter_map(|window| window.downcast::<BrowserWindow>().ok());
        browsers.collect()
    }

    fn close_all_windows() {
        for window in browser_windows() {
            window.close();
        }
        settle();
    }

    fn location(path: &Path) -> gio::File {
        gio::File::for_path(path)
    }

    #[gtk::test]
    fn launching_again_presents_the_open_window_instead_of_adding_one() {
        let (desktop, _settings) = desktop();
        desktop.activate(&application());
        desktop.activate(&application());
        assert_eq!(browser_windows().len(), 1);
        close_all_windows();
    }

    #[gtk::test]
    fn a_new_window_starts_in_the_home_folder() {
        let (desktop, _settings) = desktop();
        desktop.activate(&application());
        let [window] = &browser_windows()[..] else {
            panic!("one window is open");
        };
        let home = file_uri(&glib::home_dir());
        assert_eq!(window.current_uri(), Some(home));
        close_all_windows();
    }

    #[gtk::test]
    fn opened_folders_go_to_the_active_window_first_tab_first() {
        let (desktop, _settings) = desktop();
        let fixture = Fixture::standard();
        desktop.activate(&application());
        let files = [location(&fixture.path("Documents")), location(fixture.root())];
        desktop.open(&application(), &files);
        let [window] = &browser_windows()[..] else {
            panic!("the open window takes the locations");
        };
        wait_until("both folders to open", || window.tab_count() == 2);
        assert_eq!(
            window.current_uri(),
            Some(fixture.uri()),
            "the last folder opens in front"
        );
        WidgetExt::activate_action(window, "win.previous-tab", None).expect("tab actions exist");
        assert_eq!(window.current_uri(), Some(fixture.uri_of("Documents")));
        close_all_windows();
    }

    /// parity: TAB-042
    #[gtk::test]
    fn ctrl_n_opens_another_window_at_the_current_folder() {
        let (desktop, _settings) = desktop();
        let fixture = Fixture::standard();
        desktop.activate(&application());
        desktop.open(&application(), &[location(fixture.root())]);
        let first = browser_windows()[0].clone();
        wait_until("the folder to open", || {
            first.current_uri() == Some(fixture.uri())
        });
        first.present();
        desktop.new_window(&application());
        let windows = browser_windows();
        assert_eq!(windows.len(), 2);
        let second = windows
            .iter()
            .find(|window| **window != first)
            .expect("a second window");
        assert_eq!(second.current_uri(), Some(fixture.uri()));
        close_all_windows();
    }

    /// parity: TAB-050
    #[gtk::test]
    fn closing_one_window_releases_it_while_another_stays_open() {
        let (desktop, _settings) = desktop();
        let first = desktop.open_window(&application(), None).downgrade();
        let second = desktop.open_window(&application(), None).downgrade();
        first.upgrade().expect("the first window is open").close();
        settle();
        assert!(first.upgrade().is_none(), "a closed window is released");
        assert!(second.upgrade().is_some());
        second.upgrade().expect("the second window is open").close();
        settle();
        assert!(second.upgrade().is_none());
        assert!(browser_windows().is_empty());
    }
}
