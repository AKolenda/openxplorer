// SPDX-License-Identifier: AGPL-3.0-only
//! What every window test shares: one registered application and skin per
//! test process, and per test a fresh window with its own settings file and
//! folder fixture.
//!
//! Tests run on GTK's test thread (`#[gtk::test]`), which owns the default
//! main context, so waiting means iterating that context here.

use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use ox_core::location::file_uri;
use ox_core::settings::{Settings, Theme};
use tempfile::TempDir;

use crate::app_context::AppContext;
use crate::application::AppAction;
use crate::theme::Skin;
use crate::window::BrowserWindow;

/// How long a test waits for the window to settle before it fails.
pub(crate) const WAIT_LIMIT: Duration = Duration::from_secs(8);

/// How often a waiting test checks its condition again.
const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Frames a capture waits for, so the layout has settled.
const CAPTURE_SETTLE_FRAMES: u32 = 3;

/// The standard fixture's visible items, as the details view sorts them:
/// folders first, then names in natural order.
pub(crate) const STANDARD_NAMES: [&str; 4] = ["Documents", "Notes 2.txt", "Notes 10.txt", "Résumé.txt"];

/// The application and skin of the test process.
#[derive(Debug)]
struct TestProcess {
    app: gtk::Application,
    skin: Skin,
}

impl TestProcess {
    /// Registers the test application on the private session bus and
    /// installs the skin on the private display.
    fn new() -> Self {
        let app = gtk::Application::builder()
            .application_id("io.winspace.Development.Native.Test")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>)
            .expect("the private session bus accepts the test application");
        crate::window::install_accelerators(&app);
        add_inert_app_actions(&app);
        let display = gdk::Display::default()
            .expect("window tests run on a private display: use native/tools/check.py");
        let skin = Skin::install(&display);
        Self { app, skin }
    }
}

/// Adds the application actions windows name (`app.new-window`, ...) to
/// the test application as actions that do nothing, so the buttons that run
/// them are enabled as in the real application, and a test never opens a
/// window it did not ask for.
fn add_inert_app_actions(app: &gtk::Application) {
    let actions = [AppAction::NewWindow, AppAction::Quit];
    for action in actions {
        app.add_action(&gio::SimpleAction::new(action.name(), None));
    }
}

thread_local! {
    /// Created on first use by GTK's test thread, which runs every window test.
    static TEST_PROCESS: TestProcess = TestProcess::new();
}

/// The test application.
pub(crate) fn application() -> gtk::Application {
    TEST_PROCESS.with(|process| process.app.clone())
}

/// The skin every test window shares.
pub(crate) fn skin() -> Skin {
    TEST_PROCESS.with(|process| process.skin.clone())
}

/// Handles everything already queued on the main loop.
pub(crate) fn settle() {
    let context = glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

/// Runs the main loop until `condition` holds.
///
/// # Panics
///
/// When it does not hold within [`WAIT_LIMIT`]; the message names `what`.
pub(crate) fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT_LIMIT;
    loop {
        settle();
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(POLL_INTERVAL);
    }
}

/// Runs the main loop for `duration`, for tests that prove something does
/// not happen.
pub(crate) fn wait_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        settle();
        thread::sleep(POLL_INTERVAL);
    }
}

/// Every descendant of `widget` of type `T`, in tree order.
pub(crate) fn descendants<T: IsA<gtk::Widget>>(widget: &impl IsA<gtk::Widget>) -> Vec<T> {
    let mut found = Vec::new();
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Ok(matching) = current.clone().downcast::<T>() {
            found.push(matching);
        }
        found.extend(descendants::<T>(&current));
        child = current.next_sibling();
    }
    found
}

/// A folder tree in a temporary directory, deleted when dropped.
#[derive(Debug)]
pub(crate) struct Fixture {
    /// Owns the temporary directory, which is deleted with the fixture.
    _directory: TempDir,
    /// "Example projects" inside it.
    root: PathBuf,
}

impl Fixture {
    /// An empty "Example projects" folder.
    pub(crate) fn empty() -> Self {
        let directory = tempfile::tempdir().expect("the test home has room for fixtures");
        let root = directory.path().join("Example projects");
        fs::create_dir_all(&root).expect("fixture folder");
        Self {
            _directory: directory,
            root,
        }
    }

    /// "Example projects": a Documents folder, three text files and a
    /// hidden file.
    pub(crate) fn standard() -> Self {
        let fixture = Self::empty();
        fs::create_dir(fixture.path("Documents")).expect("fixture subfolder");
        for name in ["Notes 10.txt", "Notes 2.txt", "Résumé.txt", ".private"] {
            fixture.write(name);
        }
        fixture
    }

    /// A folder of `count` files, enough to scroll.
    pub(crate) fn with_files(count: usize) -> Self {
        let fixture = Self::empty();
        for number in 0..count {
            fixture.write(&format!("file {number:04}.txt"));
        }
        fixture
    }

    /// Creates a small file called `name` in the folder.
    pub(crate) fn write(&self, name: &str) {
        fs::write(self.path(name), b"Synthetic test data\n").expect("fixture file");
    }

    /// The path of `name` in the folder.
    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// The folder itself.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// The folder's canonical URI, as tabs store it.
    pub(crate) fn uri(&self) -> String {
        file_uri(&self.root)
    }

    /// The canonical URI of `name` in the folder.
    pub(crate) fn uri_of(&self, name: &str) -> String {
        file_uri(&self.path(name))
    }
}

/// A window of the test application, closed when dropped.
#[derive(Debug)]
pub(crate) struct TestWindow {
    /// The window under test.
    pub(crate) window: BrowserWindow,
    /// Its application state, shared with windows opened beside it.
    pub(crate) context: AppContext,
    /// Kept alive while the window reads and writes settings there.
    settings_directory: Rc<TempDir>,
}

impl TestWindow {
    /// A window with its own settings file, showing `uri` once listed.
    /// Opening files is recorded instead of starting applications.
    pub(crate) fn open(uri: &str) -> Self {
        Self::open_with_skin(uri, &skin())
    }

    /// A window like [`Self::open`] that follows `skin` instead of the
    /// shared one, for tests that watch what a window connects to it.
    pub(crate) fn open_with_skin(uri: &str, skin: &Skin) -> Self {
        let test = Self::with_skin(skin);
        test.show(uri);
        test
    }

    /// A window with its own settings file and no tab yet, for tests that
    /// watch the first listing.
    pub(crate) fn without_tabs() -> Self {
        Self::with_skin(&skin())
    }

    /// A window like [`Self::open`] whose shared state `prepare` sets up
    /// before the window uses it, such as a simulated update service or an
    /// in-memory table of default applications.
    pub(crate) fn open_prepared(uri: &str, prepare: impl FnOnce(&AppContext)) -> Self {
        let test = Self::prepared(&skin(), prepare);
        test.show(uri);
        test
    }

    /// A window with its own settings file and no tab yet, following
    /// `skin`.
    fn with_skin(skin: &Skin) -> Self {
        Self::prepared(skin, |_| {})
    }

    /// A window following `skin`, with no tab yet, whose shared state
    /// `prepare` sets up first.
    fn prepared(skin: &Skin, prepare: impl FnOnce(&AppContext)) -> Self {
        let settings = tempfile::tempdir().expect("the test home has room for settings");
        let context = AppContext::new(skin.clone(), Settings::open(settings.path()));
        context.record_launches();
        prepare(&context);
        // The context menus list no "Open in <editor>", whatever editors
        // the machine running the tests has.
        context.desktop_integration().use_editor_shortcuts(Vec::new());
        Self {
            window: BrowserWindow::new(&application(), &context),
            context,
            settings_directory: Rc::new(settings),
        }
    }

    /// A second window sharing this window's application state.
    pub(crate) fn open_beside(&self, uri: &str) -> Self {
        let beside = Self {
            window: BrowserWindow::new(&application(), &self.context),
            context: self.context.clone(),
            settings_directory: Rc::clone(&self.settings_directory),
        };
        beside.show(uri);
        beside
    }

    /// Adds a tab for `uri`, presents the window and waits for the listing.
    pub(crate) fn show(&self, uri: &str) {
        self.window
            .add_tab(uri)
            .expect("test locations are valid addresses");
        self.window.present();
        wait_until("the first listing", || {
            self.window.is_mapped() && !self.window.is_loading()
        });
    }

    /// The names shown, in display order.
    pub(crate) fn names(&self) -> Vec<String> {
        let model = self.window.folder_model();
        (0..model.n_items())
            .filter_map(|position| model.name_at(position))
            .collect()
    }

    /// The position of the item called `name` in the view.
    ///
    /// # Panics
    ///
    /// When no item of that name is listed.
    pub(crate) fn position_of(&self, name: &str) -> u32 {
        let model = self.window.folder_model();
        (0..model.n_items())
            .find(|position| model.name_at(*position).as_deref() == Some(name))
            .unwrap_or_else(|| panic!("{name} is listed"))
    }

    /// The names of the selected items, in display order.
    pub(crate) fn selected_names(&self) -> Vec<String> {
        let items = self.window.folder_model().selected_items();
        items.iter().map(|item| item.entry().name.clone()).collect()
    }

    /// Runs the window action `name` (`win.` omitted) with a string target.
    pub(crate) fn activate(&self, name: &str, target: Option<&str>) {
        let target = target.map(ToVariant::to_variant);
        WidgetExt::activate_action(&self.window, &format!("win.{name}"), target.as_ref())
            .expect("the window has the action");
    }

    /// The string state of the window action `name`.
    pub(crate) fn action_state(&self, name: &str) -> Option<String> {
        let state = self.window.lookup_action(name)?.state()?;
        state.get::<String>()
    }

    /// Waits until the active tab has finished listing.
    pub(crate) fn wait_for_listing(&self, what: &str) {
        wait_until(what, || !self.window.is_loading());
    }

    /// The directory of the window's settings file, for tests that change
    /// it as another process would.
    pub(crate) fn settings_directory(&self) -> &Path {
        self.settings_directory.path()
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        self.window.close();
        // The search cache's thread would otherwise tick on in a settings
        // directory that is about to be deleted.
        self.context.search_cache().shut_down();
        settle();
    }
}

/// The browser windows open now other than `known`.
pub(crate) fn windows_besides(known: &[&BrowserWindow]) -> Vec<BrowserWindow> {
    let windows = application().windows();
    windows
        .into_iter()
        .filter_map(|window| window.downcast::<BrowserWindow>().ok())
        .filter(|window| !known.contains(&window))
        .collect()
}

/// A window the app opened by itself during a test, such as a torn-out
/// tab's, closed when dropped even after a failed assertion.
#[derive(Debug)]
pub(crate) struct OpenedWindows(Vec<BrowserWindow>);

impl OpenedWindows {
    /// The one window opened besides `known`, once there is one.
    ///
    /// # Panics
    ///
    /// When none opens within [`WAIT_LIMIT`], or more than one does.
    pub(crate) fn only(known: &[&BrowserWindow]) -> Self {
        wait_until("a new window", || !windows_besides(known).is_empty());
        let opened = windows_besides(known);
        assert_eq!(opened.len(), 1, "exactly one window opens");
        OpenedWindows(opened)
    }

    /// The window that opened.
    pub(crate) fn window(&self) -> &BrowserWindow {
        &self.0[0]
    }
}

impl Drop for OpenedWindows {
    fn drop(&mut self) {
        for window in &self.0 {
            window.close();
        }
        settle();
    }
}

/// Keeps the shared skin's theme choice for the length of a test that
/// changes it.
#[derive(Debug)]
pub(crate) struct ThemeGuard(Theme);

impl ThemeGuard {
    /// Remembers the current choice; dropping the guard restores it.
    pub(crate) fn keep() -> Self {
        Self(skin().theme())
    }
}

impl Drop for ThemeGuard {
    fn drop(&mut self) {
        skin().set_theme(self.0);
    }
}

/// The directory named by `$OX_NATIVE_CAPTURE_DIR`, created, or `None`
/// when captures are off.
fn capture_directory() -> Option<PathBuf> {
    let directory = PathBuf::from(std::env::var_os("OX_NATIVE_CAPTURE_DIR")?);
    fs::create_dir_all(&directory).expect("capture directory");
    Some(directory)
}

/// Saves a PNG of `window` into `$OX_NATIVE_CAPTURE_DIR`, when set, for
/// visual review of the layout.
pub(crate) fn capture(window: &BrowserWindow, filename: &str) {
    let Some(directory) = capture_directory() else {
        return;
    };
    wait_for_frames(window, CAPTURE_SETTLE_FRAMES);
    let path = directory.join(filename);
    // A window that was just resized may not be drawable for a frame or two.
    wait_until("the window to be saved", || {
        crate::snapshot::save_png(window.upcast_ref(), &path).is_ok()
    });
}

/// Saves a PNG of the open `popover` into `$OX_NATIVE_CAPTURE_DIR`, when
/// set. A popover is a surface of its own, which a window capture leaves
/// out.
pub(crate) fn capture_popover(window: &BrowserWindow, popover: &gtk::Popover, filename: &str) {
    let Some(directory) = capture_directory() else {
        return;
    };
    wait_for_frames(window, CAPTURE_SETTLE_FRAMES);
    crate::snapshot::render_png(popover, None, &directory.join(filename))
        .expect("an open popover is drawn and the capture directory is writable");
}

/// Saves a PNG of the whole `dialog`, its shadow included, into
/// `$OX_NATIVE_CAPTURE_DIR`, when set. A dialog is a window of its own,
/// which a capture of the browser window leaves out.
pub(crate) fn capture_dialog(dialog: &impl IsA<gtk::Window>, filename: &str) {
    let Some(directory) = capture_directory() else {
        return;
    };
    let dialog = dialog.upcast_ref::<gtk::Window>();
    wait_for_frames(dialog, CAPTURE_SETTLE_FRAMES);
    let path = directory.join(filename);
    // A dialog that was just shown may not be drawable for a frame or two.
    wait_until("the dialog to be saved", || {
        crate::snapshot::render_png(dialog, None, &path).is_ok()
    });
}

/// Waits until `window` has drawn `count` frames, so a capture shows the
/// finished layout.
pub(crate) fn wait_for_frames(window: &impl IsA<gtk::Widget>, count: u32) {
    let frames = Rc::new(Cell::new(0));
    let counter = Rc::clone(&frames);
    window.add_tick_callback(move |_, _| {
        counter.set(counter.get() + 1);
        if counter.get() >= count {
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
    wait_until("the window to draw", || frames.get() >= count);
}
