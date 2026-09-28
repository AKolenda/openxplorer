// SPDX-License-Identifier: AGPL-3.0-only
//! What the running instance does for a command line, a launcher quick
//! action or a `FileManager1` request.
//!
//! Ports `command_line`, `show_windows`, `open_settings`, `quit_safely`
//! and `handle_reveal` of `OpenXplorer` in `desktop/winspace.py`
//! (INT-004, INT-006, INT-007, INT-014, INT-017, INT-023, SET-002,
//! TAB-052).

use gtk::glib;
use gtk::prelude::*;
use ox_core::integration::{FileManagerMethod, FileManagerRequest};

use super::command_line::CommandRequest;
use super::state::{active_window, open_window, AppState};
use crate::app_context::AppContext;
use crate::settings_page::SettingsView;
use crate::window::{BrowserWindow, QUIT_WHILE_WRITING};

impl AppState {
    /// Does what `request` asks.
    pub(super) fn run_command(&self, app: &gtk::Application, request: CommandRequest) -> glib::ExitCode {
        match request {
            CommandRequest::Quit => {
                let quit = self.quit_safely(app);
                return if quit {
                    glib::ExitCode::SUCCESS
                } else {
                    glib::ExitCode::FAILURE
                };
            }
            CommandRequest::FileManagerService => {
                self.context().desktop_integration().start_service_if_enabled();
            }
            CommandRequest::Select(files) => self.select(&files),
            CommandRequest::Windows => self.show_windows(app),
            CommandRequest::Settings => self.open_settings(app),
            CommandRequest::NewWindow(locations) => self.new_window_at(app, locations),
            CommandRequest::Open(locations) => self.open_in_active_window(app, locations),
            CommandRequest::Activate => self.activate(app),
        }
        glib::ExitCode::SUCCESS
    }

    /// `--select`: each file's folder opens with the file selected, as
    /// `FileManager1.ShowItems` does (INT-007).
    fn select(&self, files: &[String]) {
        let integration = self.context().desktop_integration();
        match FileManagerRequest::new(FileManagerMethod::ShowItems, files) {
            Ok(request) => {
                // A request the app cannot show was already reported in
                // the window that took it.
                let _ = integration.show_request(request);
            }
            Err(error) => glib::g_warning!(ox_core::LOG_DOMAIN, "--select: {error}"),
        }
    }

    /// Lists the open windows in the active one, or opens the first
    /// (`show_windows`).
    pub(super) fn show_windows(&self, app: &gtk::Application) {
        let window = active_window(app).unwrap_or_else(|| open_window(app, self.context(), None));
        window.present();
        window.show_open_windows();
    }

    /// Opens Settings in the active window, or in a new window when none
    /// is open (`open_settings`, SET-002).
    pub(super) fn open_settings(&self, app: &gtk::Application) {
        let window = active_window(app).unwrap_or_else(|| open_window(app, self.context(), None));
        window.present();
        window.open_settings(None::<SettingsView>);
    }

    /// `--new-window`: a window at the first location, or home, with the
    /// other locations as tabs (INT-006).
    fn new_window_at(&self, app: &gtk::Application, locations: Vec<String>) {
        if let Some(refusal) = self.context().updates().new_window_refusal() {
            report_in_active_window(app, &refusal);
            return;
        }
        let mut locations = locations.into_iter();
        let window = open_window(app, self.context(), locations.next().as_deref());
        let rest: Vec<String> = locations.collect();
        if !rest.is_empty() {
            window.open_locations_as_tabs(rest);
        }
    }

    /// Locations from the command line: they open in the active window,
    /// or a new one when none is open.
    fn open_in_active_window(&self, app: &gtk::Application, locations: Vec<String>) {
        let window = active_window(app).unwrap_or_else(|| open_window(app, self.context(), None));
        window.present();
        window.open_locations(locations);
    }

    /// Quit `OpenXplorer`: every window closes and the Show in folder
    /// service stops, unless an update is installing or a window writes
    /// files (`quit_safely`, TAB-052). Returns whether the application
    /// quits.
    pub(super) fn quit_safely(&self, app: &gtk::Application) -> bool {
        if let Some(refusal) = self.context().updates().quit_refusal() {
            report_in_every_window(app, &refusal);
            return false;
        }
        // Data safety: Quit never cuts off a write, in any window.
        if browser_windows(app).any(|window| window.has_running_write()) {
            report_in_every_window(app, QUIT_WHILE_WRITING);
            return false;
        }
        for window in app.windows() {
            window.close();
        }
        if !app.windows().is_empty() {
            return false;
        }
        self.context().desktop_integration().stop_file_manager_service();
        app.quit();
        true
    }
}

/// Shows a `FileManager1` request in the active window, or a new one
/// (`handle_reveal`): the caller's startup ID first, so the desktop lets
/// the window take focus (INT-023), then the folders or items.
pub(super) fn show_file_manager_request(
    app: &gtk::Application,
    context: &AppContext,
    request: &FileManagerRequest,
    startup_id: &str,
) {
    let window = active_window(app).unwrap_or_else(|| open_window(app, context, None));
    if !startup_id.is_empty() {
        window.set_startup_id(startup_id);
    }
    window.present();
    window.show_file_manager_request(request);
}

/// Shows `message` in the active window, if there is one.
fn report_in_active_window(app: &gtk::Application, message: &str) {
    if let Some(window) = active_window(app) {
        window.show_message(message);
    }
}

/// Shows `message` in every browser window (`broadcast('notice')`).
fn report_in_every_window(app: &gtk::Application, message: &str) {
    for window in browser_windows(app) {
        window.show_message(message);
    }
}

/// The browser windows of `app`.
fn browser_windows(app: &gtk::Application) -> impl Iterator<Item = BrowserWindow> {
    let windows = app.windows();
    windows
        .into_iter()
        .filter_map(|window| window.downcast::<BrowserWindow>().ok())
}
