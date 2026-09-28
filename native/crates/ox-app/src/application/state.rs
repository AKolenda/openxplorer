// SPDX-License-Identifier: AGPL-3.0-only
//! What the application does with its windows: open the first one, present
//! it again, open locations in it and open another one.
//!
//! Ports `activate_app`, `open_files` and `create_window` in
//! `desktop/winspace.py` and `newWindow` in `desktop/ui/app.js`, and keeps
//! the skin in step with the desktop's colour scheme and contrast for as
//! long as the application runs. [`AppState`] works on any
//! `GtkApplication`, so the tests drive it on the shared test application.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{self, file_uri, VirtualPlace};
use ox_core::settings::Settings;

use crate::shared::AppContext;
use crate::snapshot::SnapshotRequest;
use crate::text_size::TextSize;
use crate::theme::contrast::ContrastSetting;
use crate::theme::system::SystemScheme;
use crate::theme::{Skin, ThemePreference};
use crate::window::BrowserWindow;

/// What lives as long as the application: the shared state and the
/// watches on the desktop's colour scheme and contrast.
#[derive(Debug)]
pub(super) struct AppState {
    context: AppContext,
    /// Kept alive so the skin follows the desktop's light or dark scheme.
    _system_scheme: Rc<SystemScheme>,
    /// Kept alive so the skin follows the desktop's high-contrast setting.
    _contrast_setting: ContrastSetting,
}

impl AppState {
    /// Installs the skin on the default display and starts watching the
    /// desktop's colour scheme. `None` without a display.
    pub(super) fn new(app: &gtk::Application, settings: Settings) -> Option<Self> {
        let display = gtk::gdk::Display::default()?;
        let skin = Skin::install(&display);
        Some(Self::with_skin(app, skin, settings))
    }

    /// The application state around an installed `skin`.
    fn with_skin(app: &gtk::Application, skin: Skin, settings: Settings) -> Self {
        let preferences = &settings.data().preferences;
        skin.set_preference(ThemePreference::parse(&preferences.theme));
        skin.set_text_size(TextSize::from_percent(preferences.text_size));
        let system_scheme = follow_system_scheme(&skin);
        let contrast_setting = follow_contrast(&skin);
        crate::window::install_accelerators(app);
        Self {
            context: AppContext::new(skin, settings),
            _system_scheme: system_scheme,
            _contrast_setting: contrast_setting,
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
            // A window never opens empty. The home page name resolves
            // without the location check, which could refuse the home
            // folder's own path (BrowserWindow::resolve_address).
            window
                .add_tab(VirtualPlace::Home.uri())
                .expect("the home page name always resolves to the home folder");
        }
        if let Some(warning) = self.context.settings_warning() {
            window.notify(&warning);
        }
        window
    }

    /// Opens the window `request` describes, in its theme, size and view.
    pub(super) fn open_snapshot_window(
        &self,
        app: &gtk::Application,
        request: &SnapshotRequest,
    ) -> BrowserWindow {
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
        window
    }

    /// Presents the open window, or opens the first one.
    pub(super) fn activate(&self, app: &gtk::Application) {
        match active_window(app) {
            Some(window) => window.present(),
            None => {
                self.open_window(app, None);
            }
        }
    }

    /// Opens `files` in the active window, as `open_files` does.
    pub(super) fn open(&self, app: &gtk::Application, files: &[gio::File]) {
        let window = active_window(app).unwrap_or_else(|| self.open_window(app, None));
        let uris = files.iter().map(|file| file.uri().to_string()).collect();
        window.open_locations(uris);
        window.present();
    }

    /// Ctrl+N: another window at the active folder when it is a real
    /// folder, else at home (app.js `newWindow`).
    pub(super) fn new_window(&self, app: &gtk::Application) {
        let current = active_window(app).and_then(|window| window.current_uri());
        let start = current.filter(|uri| is_real_folder(uri));
        self.open_window(app, start.as_deref());
    }
}

/// True for a location a new window can start in (`newWindow` in app.js):
/// a local or SMB folder, not a landing page or a device.
fn is_real_folder(uri: &str) -> bool {
    matches!(location::scheme(uri).as_str(), "file" | "smb")
}

/// The focused browser window, else the most recent one.
pub(super) fn active_window(app: &gtk::Application) -> Option<BrowserWindow> {
    let focused = app.active_window().and_downcast::<BrowserWindow>();
    focused.or_else(|| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<BrowserWindow>().ok())
    })
}

/// Applies the desktop's light or dark scheme to `skin` now and on every
/// change.
fn follow_system_scheme(skin: &Skin) -> Rc<SystemScheme> {
    let scheme = SystemScheme::new(glib::clone!(
        #[weak]
        skin,
        move |appearance| skin.set_desktop_appearance(appearance)
    ));
    skin.set_desktop_appearance(scheme.appearance());
    scheme
}

/// Applies the desktop's contrast to `skin` now and on every change.
fn follow_contrast(skin: &Skin) -> ContrastSetting {
    let setting = ContrastSetting::watch(glib::clone!(
        #[weak]
        skin,
        move |contrast| skin.set_contrast(contrast)
    ));
    skin.set_contrast(setting.contrast());
    setting
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;
    use crate::test_support::harness::{application, settle, skin, wait_until, Fixture};
    use crate::theme::contrast::Contrast;

    /// Application state on the shared test application, with its own
    /// settings.
    fn app_state() -> (AppState, TempDir) {
        let settings = tempfile::tempdir().expect("the test home has room for settings");
        let state = AppState::with_skin(&application(), skin(), Settings::open(settings.path()));
        state.context.record_launches();
        (state, settings)
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

    /// The web app's `prefers-contrast: more` rules follow GNOME's
    /// accessibility setting; without its schema the contrast stays normal.
    #[gtk::test]
    fn the_skin_follows_the_desktop_high_contrast_setting() {
        let (_state, _settings) = app_state();
        let schema = gio::SettingsSchemaSource::default()
            .and_then(|source| source.lookup("org.gnome.desktop.a11y.interface", true));
        let Some(schema) = schema else {
            assert_eq!(skin().contrast(), Contrast::Normal);
            return;
        };
        let accessibility = gio::Settings::new_full(&schema, None::<&gio::SettingsBackend>, None);
        accessibility
            .set_boolean("high-contrast", true)
            .expect("the test settings backend is writable");
        wait_until("the high-contrast rules", || skin().contrast() == Contrast::High);
        accessibility
            .set_boolean("high-contrast", false)
            .expect("the test settings backend is writable");
        wait_until("the normal rules", || skin().contrast() == Contrast::Normal);
    }

    #[gtk::test]
    fn launching_again_presents_the_open_window_instead_of_adding_one() {
        let (state, _settings) = app_state();
        state.activate(&application());
        state.activate(&application());
        assert_eq!(browser_windows().len(), 1);
        close_all_windows();
    }

    /// parity: TAB-042
    #[gtk::test]
    fn a_new_window_starts_in_the_home_folder() {
        let (state, _settings) = app_state();
        state.activate(&application());
        let [window] = &browser_windows()[..] else {
            panic!("one window is open");
        };
        let home = file_uri(&glib::home_dir());
        assert_eq!(window.current_uri(), Some(home));
        close_all_windows();
    }

    #[gtk::test]
    fn opened_folders_go_to_the_active_window_first_tab_first() {
        let (state, _settings) = app_state();
        let fixture = Fixture::standard();
        state.activate(&application());
        let files = [location(&fixture.path("Documents")), location(fixture.root())];
        state.open(&application(), &files);
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
        let (state, _settings) = app_state();
        let fixture = Fixture::standard();
        state.activate(&application());
        state.open(&application(), &[location(fixture.root())]);
        let first = browser_windows()[0].clone();
        wait_until("the folder to open", || {
            first.current_uri() == Some(fixture.uri())
        });
        first.present();
        state.new_window(&application());
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
        let (state, _settings) = app_state();
        let first = state.open_window(&application(), None).downgrade();
        let second = state.open_window(&application(), None).downgrade();
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
