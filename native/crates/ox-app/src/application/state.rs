// SPDX-License-Identifier: AGPL-3.0-only
//! What the application does with its windows: open the first one, present
//! it again, open locations in it and open another one.
//!
//! Ports `activate_app`, `open_files` and `create_window` in
//! `v2.0.0:desktop/winspace.py` and `newWindow` in `v2.0.0:desktop/ui/app.js`, and keeps
//! the skin in step with the desktop's colour scheme and contrast for as
//! long as the application runs. It also attaches the desktop
//! integration, whose `FileManager1` requests open in the active window
//! ([`super::requests`]). [`AppState`] works on any `GtkApplication`, so
//! the tests drive it on the shared test application.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use ox_core::location::{file_uri, location_kind, LocationKind, VirtualPlace};
use ox_core::session::SavedSession;
use ox_core::settings::{Appearance, Settings};

use super::clock_format::ClockSetting;
use crate::app_context::AppContext;
use crate::search::CacheLocation;
use crate::snapshot::SnapshotRequest;
use crate::text_size::TextSize;
use crate::theme::accent::AccentSetting;
use crate::theme::contrast::ContrastSetting;
use crate::theme::desktop_text::DesktopTextWatch;
use crate::theme::system::{self, SystemScheme};
use crate::theme::Skin;
use crate::window::BrowserWindow;

/// The host's configuration folder, where the desktop reads its places
/// list. Inside Flatpak, `GLib` names the sandbox's own folder, which the
/// host never reads; the sandbox may write the host's `~/.config`
/// (`--filesystem=host`), so the list goes there. A custom
/// `XDG_CONFIG_HOME` on the host is not visible from inside the sandbox.
#[cfg(not(test))]
fn host_config_dir() -> std::path::PathBuf {
    if ox_core::integration::Sandbox::detect().is_flatpak() {
        glib::home_dir().join(".config")
    } else {
        glib::user_config_dir()
    }
}

/// What lives as long as the application: the shared state and the
/// watches on the desktop's colour scheme and contrast.
#[derive(Debug)]
pub(super) struct AppState {
    context: AppContext,
    /// Kept alive so the skin follows the desktop's light or dark scheme.
    _system_scheme: Rc<SystemScheme>,
    /// Kept alive so the skin follows the desktop's high-contrast setting.
    _contrast_setting: ContrastSetting,
    /// Kept alive so text follows the desktop's font and text scaling.
    _desktop_text: Option<DesktopTextWatch>,
    /// Kept alive so the skin follows the desktop's accent colour.
    _accent_setting: AccentSetting,
    /// Kept alive so Properties timestamps follow the desktop's clock.
    _clock_setting: ClockSetting,
}

impl AppState {
    /// Installs the skin on the default display and starts watching the
    /// desktop's colour scheme. `None` without a display.
    pub(super) fn new(app: &gtk::Application, settings: Settings) -> Option<Self> {
        let display = gtk::gdk::Display::default()?;
        // Installing the skin forces GTK's dark preference off, so it is
        // read first.
        let gtk_preference = system::gtk_preference(&display);
        let skin = Skin::install(&display);
        Some(Self::with_skin(app, skin, gtk_preference, settings))
    }

    /// The application state around an installed `skin`. `gtk_preference`
    /// is GTK's dark preference from before the skin was installed, which
    /// the desktop's appearance falls back to.
    fn with_skin(app: &gtk::Application, skin: Skin, gtk_preference: Appearance, settings: Settings) -> Self {
        let preferences = &settings.data().preferences;
        skin.set_theme(preferences.theme);
        skin.set_uses_desktop_font(preferences.desktop_font);
        skin.set_text_size(TextSize::from_percent(preferences.text_size));
        let desktop_text = follow_desktop_text(&skin);
        let system_scheme = follow_system_scheme(&skin, gtk_preference);
        let contrast_setting = follow_contrast(&skin);
        let accent_setting = follow_accent(&skin);
        let clock_setting = ClockSetting::follow();
        crate::window::install_accelerators(app);
        let context = AppContext::new(skin, settings);
        context.start_search_cache(CacheLocation::UserCache);
        // Tests mirror pins into temporary lists only, never the user's.
        #[cfg(not(test))]
        context.export_pins_to(ox_core::places::bookmarks_file(&host_config_dir()));
        attach_desktop_integration(app, &context);
        Self {
            context,
            _system_scheme: system_scheme,
            _contrast_setting: contrast_setting,
            _desktop_text: desktop_text,
            _accent_setting: accent_setting,
            _clock_setting: clock_setting,
        }
    }

    /// Stops what outlives the windows and must not outlive the app: the
    /// search index service.
    pub(super) fn shut_down(&self) {
        self.context.search_cache().shut_down();
    }

    /// What every window of the application shares.
    pub(super) fn context(&self) -> &AppContext {
        &self.context
    }

    /// Opens a window whose first tab shows `start`, or the startup folder.
    fn open_window(&self, app: &gtk::Application, start: Option<&str>) -> BrowserWindow {
        open_window(app, &self.context, start)
    }

    /// A window whose first tab shows `start`, or the startup folder, not
    /// shown yet.
    fn build_window(&self, app: &gtk::Application, start: Option<&str>) -> BrowserWindow {
        build_window(app, &self.context, start)
    }

    /// Opens the window `request` describes, in its theme, size and view,
    /// with Settings open at its page and search when it asks for them.
    pub(super) fn open_snapshot_window(
        &self,
        app: &gtk::Application,
        request: &SnapshotRequest,
    ) -> BrowserWindow {
        if let Some(theme) = request.theme {
            self.context.skin().set_theme(theme);
        }
        let window = self.build_window(app, request.start.as_deref());
        if let Some(size) = request.size {
            window.set_default_size(size.width, size.height);
        }
        if let Some(view) = request.view {
            window.show_view(view);
        }
        let settings_asked = request.settings.is_some() || request.settings_search.is_some();
        if settings_asked {
            window.open_settings(request.settings);
        }
        if let Some(query) = &request.settings_search {
            window.search_settings(query);
        }
        if let Some(query) = &request.search {
            window.search_folder(query);
        }
        window.present();
        window
    }

    /// Presents the open window, lists the windows when several are open,
    /// or opens the first one (`activate_app`), with the tabs of the last
    /// session when the settings ask for them (TAB-053).
    pub(super) fn activate(&self, app: &gtk::Application) {
        let Some(window) = active_window(app) else {
            self.open_first_window(app);
            return;
        };
        if browser_windows_of(app).count() > 1 {
            self.show_windows(app);
        } else {
            window.present();
        }
    }

    /// Opens the first window of a start without locations: the tabs the
    /// last window had when the settings ask to restore them and they were
    /// saved, else a window at the startup folder.
    fn open_first_window(&self, app: &gtk::Application) {
        let preferences = self.context.settings_data().preferences;
        let saved = preferences
            .restore_session
            .then(|| SavedSession::load(&self.context.settings_directory()))
            .flatten();
        let Some(saved) = saved else {
            self.open_window(app, None);
            return;
        };
        let window = BrowserWindow::new(app, &self.context);
        window.restore_session(&saved);
        if window.tab_count() == 0 {
            // A window never opens empty.
            window.destroy();
            self.open_window(app, None);
            return;
        }
        if let Some(warning) = self.context.settings_warning() {
            window.show_message(&warning);
        }
        window.present_as_new_window();
    }

    /// `--split`: the locations paired into split tabs, in a new window
    /// when `new_window` asks for one or none is open, else as new tabs of
    /// the active window (INT-005). Without locations, the startup folder
    /// beside itself.
    pub(super) fn open_split(&self, app: &gtk::Application, locations: Vec<String>, new_window: bool) {
        let locations = if locations.is_empty() {
            vec![startup_location(&self.context)]
        } else {
            locations
        };
        let in_new_window = new_window
            || self
                .context
                .settings_data()
                .preferences
                .external_folders_in_new_window;
        if let Some(window) = active_window(app).filter(|_| !in_new_window) {
            window.present();
            window.open_split_tabs(&locations, false);
            return;
        }
        if let Some(refusal) = self.context.updates().new_window_refusal() {
            if let Some(window) = active_window(app) {
                window.show_message(&refusal);
            }
            return;
        }
        let first = locations.first().map(String::as_str);
        let window = window_with_first_tab(app, &self.context, first);
        window.open_split_tabs(&locations, true);
        window.present_as_new_window();
    }

    /// Opens `files` from `GApplication` open (xdg-open) as the command
    /// line opens its locations.
    pub(super) fn open(&self, app: &gtk::Application, files: &[gio::File]) {
        let uris = files.iter().map(|file| file.uri().to_string()).collect();
        self.open_in_active_window(app, uris);
    }

    /// Ctrl+N: another window at the active folder when it is a real
    /// folder, else at home (app.js `newWindow`). New windows are refused
    /// while an update installs and until its restart (TAB-043).
    pub(super) fn new_window(&self, app: &gtk::Application) {
        if let Some(refusal) = self.context.updates().new_window_refusal() {
            if let Some(window) = active_window(app) {
                window.show_message(&refusal);
            }
            return;
        }
        let current = active_window(app).and_then(|window| window.current_uri());
        let start = current.filter(|uri| can_start_a_new_window_in(uri));
        self.open_window(app, start.as_deref());
    }
}

/// Whether a new window may start in `uri`: a folder on this computer or
/// on a network server, rather than a landing page, a device or another
/// virtual place (`newWindow` in app.js).
fn can_start_a_new_window_in(uri: &str) -> bool {
    matches!(
        location_kind(uri),
        LocationKind::Local | LocationKind::Smb | LocationKind::Remote
    )
}

/// The focused browser window, else the most recent one. A window that
/// is another application's file dialog is never one (INT-032).
pub(super) fn active_window(app: &gtk::Application) -> Option<BrowserWindow> {
    let focused = app
        .active_window()
        .and_downcast::<BrowserWindow>()
        .filter(|window| !window.is_picking());
    focused.or_else(|| browser_windows_of(app).next())
}

/// The browser windows of `app`, most recent first, without the file
/// dialogs it shows for other applications.
pub(super) fn browser_windows_of(app: &gtk::Application) -> impl Iterator<Item = BrowserWindow> {
    let windows = app.windows();
    windows
        .into_iter()
        .filter_map(|window| window.downcast::<BrowserWindow>().ok())
        .filter(|window| !window.is_picking())
}

/// Shows another application's Open or Save dialog in a new window
/// (INT-032).
fn open_picker_window(app: &gtk::Application, context: &AppContext, call: ox_core::integration::ChooserCall) {
    let window = BrowserWindow::new(app, context);
    window.begin_picking(call);
}

/// Opens a window of `app` whose first tab shows `start`, or the startup
/// folder (`create_window`).
pub(super) fn open_window(
    app: &gtk::Application,
    context: &AppContext,
    start: Option<&str>,
) -> BrowserWindow {
    let window = build_window(app, context, start);
    window.present_as_new_window();
    window
}

/// Where a new window opens without a location: the startup folder of the
/// settings while it is a folder that exists, else the home folder
/// (TAB-055).
fn startup_location(context: &AppContext) -> String {
    let chosen = context.settings_data().preferences.startup_folder;
    let exists = |uri: &String| {
        let file = gio::File::for_uri(uri);
        // Only a local folder is checked: a share may need a sign-in, which
        // its listing asks for.
        file.path().is_none_or(|path| path.is_dir())
    };
    chosen
        .filter(exists)
        .unwrap_or_else(|| file_uri(&glib::home_dir()))
}

/// A window whose first tab shows `start`, or the startup folder, not
/// shown yet; split when the settings ask new windows to begin split
/// (VIEW-059).
fn build_window(app: &gtk::Application, context: &AppContext, start: Option<&str>) -> BrowserWindow {
    let window = window_with_first_tab(app, context, start);
    if context.settings_data().preferences.begin_in_split_view {
        if let Err(error) = window.split_tab(None) {
            window.show_message(&error.to_string());
        }
    }
    window
}

/// A window whose one tab shows `start`, or the startup folder, not shown
/// yet.
fn window_with_first_tab(app: &gtk::Application, context: &AppContext, start: Option<&str>) -> BrowserWindow {
    let window = BrowserWindow::new(app, context);
    let startup = startup_location(context);
    let start = start.unwrap_or(startup.as_str());
    if let Err(error) = window.add_tab(start) {
        window.show_message(&error.to_string());
        // A window never opens empty. The home page name resolves
        // without the location check, which could refuse the home
        // folder's own path (BrowserWindow::resolve_address).
        if let Err(error) = window.add_tab(VirtualPlace::Home.uri()) {
            window.show_message(&error.to_string());
        }
    }
    if let Some(warning) = context.settings_warning() {
        window.show_message(&warning);
    }
    window
}

/// Attaches the desktop integration to `app`: `FileManager1` requests
/// open in its active window, and the Show in folder service starts when
/// it is enabled.
fn attach_desktop_integration(app: &gtk::Application, context: &AppContext) {
    let show = glib::clone!(
        #[weak]
        app,
        #[weak]
        context,
        #[upgrade_or]
        Err(ox_core::integration::RequestNotOpened),
        move |request, startup_id: String| {
            super::requests::show_file_manager_request(&app, &context, &request, &startup_id);
            Ok(())
        }
    );
    context.desktop_integration().attach(app, show);
    let pick = glib::clone!(
        #[weak]
        app,
        #[weak]
        context,
        #[upgrade_or]
        Err(ox_core::integration::ChooserNotShown),
        move |call| {
            open_picker_window(&app, &context, call);
            Ok(())
        }
    );
    if let Some(app) = app.downcast_ref::<super::Application>() {
        app.route_file_dialogs(pick);
    }
}

/// Applies the desktop's light or dark scheme to `skin` now and on every
/// change; `gtk_preference` decides when the desktop's keys do not.
fn follow_system_scheme(skin: &Skin, gtk_preference: Appearance) -> Rc<SystemScheme> {
    let on_change = glib::clone!(
        #[weak]
        skin,
        move |appearance| skin.set_desktop_appearance(appearance)
    );
    let scheme = SystemScheme::new(gtk_preference, on_change);
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

/// Applies the desktop's font and text scaling to `skin` now and on every
/// change; `None` without a display.
fn follow_desktop_text(skin: &Skin) -> Option<DesktopTextWatch> {
    let settings = gtk::Settings::default()?;
    Some(DesktopTextWatch::new(
        &settings,
        glib::clone!(
            #[weak]
            skin,
            move |desktop| skin.set_desktop_text(desktop)
        ),
    ))
}

/// Applies the desktop's accent colour to `skin` now and on every change.
fn follow_accent(skin: &Skin) -> AccentSetting {
    let setting = AccentSetting::watch(glib::clone!(
        #[weak]
        skin,
        move |accent| skin.set_accent(accent)
    ));
    skin.set_accent(setting.accent());
    setting
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::path::Path;

    use ox_core::location::SETTINGS_URI;
    use ox_core::settings::{PreferencesUpdate, Theme};
    use tempfile::TempDir;

    use super::super::command_line::CommandRequest;
    use super::*;
    use crate::test_support::desktop_setting::DesktopSetting;
    use crate::test_support::harness::{application, settle, skin, wait_until, Fixture, ThemeGuard};
    use crate::theme::contrast::{self, Contrast};

    /// Application state on the shared test application with its own
    /// settings. Dropping it closes every window the test opened, even
    /// after a failed assertion, so the next test starts with none.
    struct TestApp {
        state: AppState,
        /// Owns the settings folder, deleted after the windows close.
        _settings: TempDir,
    }

    impl TestApp {
        fn new() -> Self {
            let settings = tempfile::tempdir().expect("the test home has room for settings");
            Self::with_settings_folder(settings)
        }

        /// Starts from a settings file whose saved theme is `theme`.
        fn with_saved_theme(theme: &str) -> Self {
            let settings = tempfile::tempdir().expect("the test home has room for settings");
            let contents = format!(r#"{{"preferences": {{"theme": "{theme}"}}}}"#);
            fs::write(settings.path().join(Settings::FILE_NAME), contents)
                .expect("the test settings folder is writable");
            Self::with_settings_folder(settings)
        }

        /// Starts from the settings in `settings`, which the app owns
        /// until it is dropped.
        fn with_settings_folder(settings: TempDir) -> Self {
            // The private test display's GTK settings do not prefer dark.
            let gtk_preference = Appearance::Light;
            let state = AppState::with_skin(
                &application(),
                skin(),
                gtk_preference,
                Settings::open(settings.path()),
            );
            state.context.record_launches();
            Self {
                state,
                _settings: settings,
            }
        }
    }

    impl Drop for TestApp {
        fn drop(&mut self) {
            close_all_windows();
        }
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

    fn gio_file(path: &Path) -> gio::File {
        gio::File::for_path(path)
    }

    /// The web app's `prefers-contrast: more` rules follow GNOME's
    /// accessibility setting; without its schema the contrast stays normal.
    /// The setting changes only where `GSettings` keeps it in memory.
    #[gtk::test]
    fn the_skin_follows_the_desktop_high_contrast_setting() {
        let _app = TestApp::new();
        let Some(accessibility) = contrast::accessibility_settings() else {
            assert_eq!(skin().contrast(), Contrast::Normal);
            return;
        };
        let Some(high_contrast) = DesktopSetting::in_memory(accessibility, contrast::HIGH_CONTRAST_KEY)
        else {
            return;
        };
        high_contrast.set_boolean(true);
        wait_until("the high-contrast rules", || skin().contrast() == Contrast::High);
        high_contrast.set_boolean(false);
        wait_until("the normal rules", || skin().contrast() == Contrast::Normal);
    }

    #[gtk::test]
    fn launching_again_presents_the_open_window_instead_of_adding_one() {
        let app = TestApp::new();
        app.state.activate(&application());
        app.state.activate(&application());
        assert_eq!(browser_windows().len(), 1);
    }

    /// parity: TAB-042
    #[gtk::test]
    fn a_new_window_starts_in_the_home_folder() {
        let app = TestApp::new();
        app.state.activate(&application());
        let [window] = &browser_windows()[..] else {
            panic!("one window is open");
        };
        let home = file_uri(&glib::home_dir());
        assert_eq!(window.current_uri(), Some(home));
    }

    #[gtk::test]
    fn opened_folders_go_to_new_tabs_of_the_active_window() {
        // Declared first so it outlives the app, whose windows show it.
        let fixture = Fixture::standard();
        let app = TestApp::new();
        app.state.activate(&application());
        let files = [gio_file(&fixture.path("Documents")), gio_file(fixture.root())];
        app.state.open(&application(), &files);
        let [window] = &browser_windows()[..] else {
            panic!("the open window takes the locations");
        };
        wait_until("both folders to open", || window.tab_count() == 3);
        assert_eq!(
            window.current_uri(),
            Some(fixture.uri()),
            "the last folder opens in front"
        );
        WidgetExt::activate_action(window, "win.previous-tab", None).expect("tab actions exist");
        assert_eq!(window.current_uri(), Some(fixture.uri_of("Documents")));
    }

    /// parity: TAB-042, TAB-043
    #[gtk::test]
    fn ctrl_n_opens_another_window_at_the_current_folder() {
        // Declared first so it outlives the app, whose windows show it.
        let fixture = Fixture::standard();
        let app = TestApp::new();
        app.state.activate(&application());
        app.state.open(&application(), &[gio_file(fixture.root())]);
        let first = browser_windows()[0].clone();
        wait_until("the folder to open", || {
            first.current_uri() == Some(fixture.uri())
        });
        first.present();
        app.state.new_window(&application());
        let windows = browser_windows();
        assert_eq!(windows.len(), 2);
        let second = windows
            .iter()
            .find(|window| **window != first)
            .expect("a second window");
        assert_eq!(second.current_uri(), Some(fixture.uri()));
    }

    /// parity: TAB-043
    #[test]
    fn a_new_window_can_start_in_a_local_or_smb_folder_only() {
        assert!(can_start_a_new_window_in("file:///home/demo"));
        assert!(can_start_a_new_window_in("smb://nas/media"));
        assert!(!can_start_a_new_window_in("ox:pc"));
        assert!(!can_start_a_new_window_in("trash:///"));
    }

    /// A saved theme and the choice the skin starts with.
    struct SavedThemeCase {
        saved: &'static str,
        theme: Theme,
    }

    /// The saved theme is the skin's choice when the application starts;
    /// settings drop an unknown value, which leaves the default, System
    /// (`applyTheme`).
    ///
    /// parity: LOOK-003
    #[gtk::test]
    fn saved_themes_parse_and_anything_else_means_system() {
        let _theme = ThemeGuard::keep();
        let cases = [
            SavedThemeCase {
                saved: "dark",
                theme: Theme::Dark,
            },
            SavedThemeCase {
                saved: "light",
                theme: Theme::Light,
            },
            SavedThemeCase {
                saved: "sepia",
                theme: Theme::System,
            },
        ];
        for case in cases {
            let _app = TestApp::with_saved_theme(case.saved);
            assert_eq!(skin().theme(), case.theme, "{}", case.saved);
        }
    }

    /// A start with Dark saved draws the dark palette before the first
    /// window exists, so that window's first frame already has the dark
    /// text colour, with no light frame before it.
    ///
    /// parity: LOOK-007
    #[gtk::test]
    fn a_saved_dark_theme_is_drawn_from_the_first_frame_of_the_first_window() {
        let _theme = ThemeGuard::keep();
        skin().set_theme(Theme::Light);
        let app = TestApp::with_saved_theme("dark");
        assert!(browser_windows().is_empty(), "no window before activation");
        assert_eq!(skin().appearance(), Appearance::Dark, "drawn before the window");

        app.state.activate(&application());
        let window = browser_windows()
            .pop()
            .expect("activation opens the first window");
        let first_frame_text = Rc::new(Cell::new(None));
        window.add_tick_callback({
            let first_frame_text = Rc::clone(&first_frame_text);
            move |window, _| {
                first_frame_text.set(Some(window.color()));
                glib::ControlFlow::Break
            }
        });
        wait_until("the first frame", || first_frame_text.get().is_some());
        let text = first_frame_text.get().expect("the first frame was seen");
        // ox_text in dark.css is #f1f1f1; light.css draws #1b1b1b.
        assert!(text.red() > 0.9 && text.blue() > 0.9, "{text:?}");
    }

    /// `--new-window` opens a window of its own at the first location, with
    /// the other locations as tabs.
    ///
    /// parity: INT-006, TAB-042
    #[gtk::test]
    fn new_window_opens_its_locations_in_a_window_of_its_own() {
        let fixture = Fixture::standard();
        let app = TestApp::new();
        app.state.activate(&application());
        let locations = vec![fixture.uri(), fixture.uri_of("Documents")];

        app.state
            .run_command(&application(), CommandRequest::NewWindow(locations));

        let windows = browser_windows();
        assert_eq!(windows.len(), 2);
        let opened = windows
            .iter()
            .find(|window| window.tab_count() == 2 || window.current_uri() == Some(fixture.uri()))
            .expect("the new window shows the locations");
        wait_until("both locations to open", || opened.tab_count() == 2);
        assert_eq!(opened.current_uri(), Some(fixture.uri_of("Documents")));
    }

    /// `--split` with two locations opens a window split between them,
    /// the second pane in front; a window opened without a location opens
    /// at the startup folder of the settings.
    ///
    /// parity: INT-005, TAB-055
    #[gtk::test]
    fn split_pairs_its_locations_and_new_windows_open_at_the_startup_folder() {
        let fixture = Fixture::standard();
        let settings = tempfile::tempdir().expect("the test home has room for settings");
        let documents = fixture.uri_of("Documents");
        let contents = format!(r#"{{"preferences": {{"startupFolder": "{documents}"}}}}"#);
        fs::write(settings.path().join(Settings::FILE_NAME), contents)
            .expect("the test settings folder is writable");
        let app = TestApp::with_settings_folder(settings);
        let locations = vec![fixture.uri(), documents.clone()];

        let split = CommandRequest::Split {
            locations,
            new_window: false,
        };
        app.state.run_command(&application(), split);
        let [window] = &browser_windows()[..] else {
            panic!("one window opens");
        };
        let shown = (window.tab_count(), window.current_uri(), window.beside_uri());
        app.state
            .run_command(&application(), CommandRequest::NewWindow(Vec::new()));

        assert_eq!(shown, (1, Some(documents.clone()), Some(fixture.uri())));
        let opened = browser_windows()
            .into_iter()
            .find(|other| other != window)
            .expect("a second window opens");
        assert_eq!(opened.current_uri(), Some(documents));
    }

    /// With no window open, the command line's locations open in the first
    /// window: the first in its tab, the others in new tabs in front.
    ///
    /// parity: NAV-041
    #[gtk::test]
    fn command_line_locations_fill_the_first_window_when_none_is_open() {
        let fixture = Fixture::standard();
        let app = TestApp::new();
        let locations = vec![fixture.uri_of("Documents"), fixture.uri()];

        app.state
            .run_command(&application(), CommandRequest::Open(locations));

        let [window] = &browser_windows()[..] else {
            panic!("one window opens");
        };
        wait_until("both locations to open", || window.tab_count() == 2);
        assert_eq!(window.current_uri(), Some(fixture.uri()));
        WidgetExt::activate_action(window, "win.previous-tab", None).expect("the action");
        assert_eq!(window.current_uri(), Some(fixture.uri_of("Documents")));
    }

    /// A folder opened from another app gets a new tab, or a new window
    /// when Settings asks for one; the tab in use stays where it was.
    ///
    /// parity: NAV-042
    #[gtk::test]
    fn folders_from_other_apps_open_in_a_new_tab_or_a_new_window() {
        let fixture = Fixture::standard();
        let app = TestApp::new();
        app.state.activate(&application());
        let [window] = &browser_windows()[..] else {
            panic!("one window opens");
        };
        let first = window.current_uri();

        app.state
            .open(&application(), &[gio::File::for_uri(&fixture.uri())]);

        wait_until("the new tab", || window.tab_count() == 2);
        assert_eq!(window.current_uri(), Some(fixture.uri()));
        WidgetExt::activate_action(window, "win.previous-tab", None).expect("the action");
        assert_eq!(window.current_uri(), first);
        let update = PreferencesUpdate {
            external_folders_in_new_window: Some(true),
            ..PreferencesUpdate::default()
        };
        app.state
            .context
            .update_preferences(update, |result| result.expect("saved"));
        wait_until("the saved option", || {
            app.state
                .context
                .settings_data()
                .preferences
                .external_folders_in_new_window
        });
        let locations = vec![fixture.uri_of("Documents")];
        app.state
            .run_command(&application(), CommandRequest::Open(locations));
        assert_eq!(browser_windows().len(), 2);
        assert_eq!(window.tab_count(), 2);
    }

    /// `--settings` and the launcher's Settings action open Settings in the
    /// open window, without a second window.
    ///
    /// parity: SET-002
    #[gtk::test]
    fn settings_opens_in_the_open_window() {
        let app = TestApp::new();
        app.state.activate(&application());

        app.state.run_command(&application(), CommandRequest::Settings);

        let [window] = &browser_windows()[..] else {
            panic!("one window stays open");
        };
        assert_eq!(window.current_uri().as_deref(), Some(SETTINGS_URI));
    }

    /// A damaged settings file opens the window on safe defaults and says
    /// so in the window.
    ///
    /// parity: SET-013
    #[gtk::test]
    fn a_damaged_settings_file_is_reported_at_startup() {
        let settings = tempfile::tempdir().expect("the test home has room for settings");
        fs::write(settings.path().join(Settings::FILE_NAME), "{ not json")
            .expect("the test settings folder is writable");
        let app = TestApp::with_settings_folder(settings);

        app.state.activate(&application());

        let [window] = &browser_windows()[..] else {
            panic!("one window opens");
        };
        let message = window.shown_message();
        assert!(
            message.starts_with("Could not fully read settings; using safe defaults."),
            "{message}"
        );
        close_all_windows();
    }

    /// Settings opens a window when none is open.
    ///
    /// parity: SET-002
    #[gtk::test]
    fn settings_opens_a_window_when_none_is_open() {
        let app = TestApp::new();

        app.state.run_command(&application(), CommandRequest::Settings);

        let [window] = &browser_windows()[..] else {
            panic!("one window opens");
        };
        assert_eq!(window.current_uri().as_deref(), Some(SETTINGS_URI));
    }

    /// `--select` shows each file in its folder, selected, as
    /// `FileManager1.ShowItems` does.
    ///
    /// parity: INT-007
    #[gtk::test]
    fn select_shows_the_file_selected_in_its_folder() {
        let fixture = Fixture::standard();
        let app = TestApp::new();
        app.state.activate(&application());

        app.state.run_command(
            &application(),
            CommandRequest::Select(vec![fixture.uri_of("Notes 2.txt")]),
        );

        let [window] = &browser_windows()[..] else {
            panic!("the open window shows the file");
        };
        wait_until("the file's folder", || {
            window.current_uri() == Some(fixture.uri())
        });
        wait_until("the selection", || {
            window.folder_model().selected_uris() == [fixture.uri_of("Notes 2.txt")]
        });
    }

    /// Quit refuses while any window writes files, and closes every window
    /// once none does.
    ///
    /// parity: TAB-052
    #[gtk::test]
    fn quit_waits_for_the_file_operations_of_every_window() {
        let app = TestApp::new();
        let idle = app.state.open_window(&application(), None);
        let writing = app.state.open_window(&application(), None);
        assert!(writing.begin_test_write());

        let quit = app.state.quit_safely(&application());

        assert!(!quit);
        assert_eq!(browser_windows().len(), 2, "no window closed");
        assert_eq!(
            idle.shown_message_text(),
            "Finish or cancel active file operations before quitting OpenXplorer."
        );
        writing.end_test_write();
        // The test application must keep running for the next test, so
        // this closes the windows as Quit would, without quitting.
        close_all_windows();
        assert!(browser_windows().is_empty());
    }

    /// With "Ask before closing a window with several tabs" on, Quit asks
    /// one question for every window with several tabs, however often it
    /// is asked, and Cancel keeps every window open.
    ///
    /// parity: SET-010, TAB-051
    #[gtk::test]
    fn quit_asks_once_about_the_tabs_of_every_window() {
        let fixture = Fixture::standard();
        let app = TestApp::new();
        let set_asking = |asks: bool| {
            let update = PreferencesUpdate {
                confirm_close_tabs: Some(asks),
                ..PreferencesUpdate::default()
            };
            app.state
                .context
                .update_preferences(update, |result| result.expect("saved"));
            wait_until("the saved option", || {
                app.state.context.settings_data().preferences.confirm_close_tabs == asks
            });
        };
        set_asking(true);
        let windows: Vec<gtk::Window> = (0..2)
            .map(|_| {
                let window = app.state.open_window(&application(), Some(&fixture.uri()));
                window.add_tab(&fixture.uri()).expect("valid folder");
                window.upcast()
            })
            .collect();
        let questions = || {
            gtk::Window::list_toplevels()
                .into_iter()
                .filter_map(|window| window.downcast::<gtk::Window>().ok())
                .filter(WidgetExt::is_visible)
                .filter(|window| {
                    window
                        .transient_for()
                        .is_some_and(|parent| windows.contains(&parent))
                })
                .collect::<Vec<_>>()
        };

        assert!(!app.state.quit_safely(&application()));
        assert!(!app.state.quit_safely(&application()));
        wait_until("the question", || !questions().is_empty());
        settle();

        let [question] = &questions()[..] else {
            panic!("one question for both windows");
        };
        assert_eq!(question.title().as_deref(), Some("Quit OpenXplorer?"));
        question.close();
        wait_until("the question to close", || questions().is_empty());
        assert_eq!(browser_windows().len(), 2, "Cancel keeps every window");
        // The windows close at the end of the test without asking.
        set_asking(false);
    }

    /// A resized window saves its size, and every new window opens at it:
    /// Ctrl+N's, Open in new window's and Move tab to new window's.
    ///
    /// parity: TAB-054
    #[gtk::test]
    fn a_new_window_opens_at_the_last_windows_size() {
        let fixture = Fixture::standard();
        let app = TestApp::new();
        let first = app.state.open_window(&application(), Some(&fixture.uri()));
        first.set_default_size(900, 640);
        wait_until("the size to be saved", || {
            app.state
                .context
                .settings_data()
                .preferences
                .window_size
                .is_some()
        });
        let opened_from = |action: &str, target: glib::Variant| {
            let before = browser_windows();
            WidgetExt::activate_action(&first, action, Some(&target)).expect("a window action");
            let after = browser_windows();
            let new = after.into_iter().find(|window| !before.contains(window));
            new.expect("the action opens a window")
        };

        let in_new_window = opened_from("win.open-window", fixture.uri_of("Documents").to_variant());
        first.add_tab(&fixture.uri()).expect("valid folder");
        let tab = first.active_tab_target().expect("a tab in front");
        let moved_tab = opened_from("win.move-tab-to-new-window", tab);
        first.close();
        settle();
        let second = app.state.open_window(&application(), None);

        for window in [&in_new_window, &moved_tab, &second] {
            assert_eq!(window.default_size(), (900, 640));
            assert!(!window.is_maximized());
        }
    }

    /// parity: TAB-050
    #[gtk::test]
    fn closing_one_window_releases_it_while_another_stays_open() {
        let app = TestApp::new();
        let first = app.state.open_window(&application(), None).downgrade();
        let second = app.state.open_window(&application(), None).downgrade();
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
