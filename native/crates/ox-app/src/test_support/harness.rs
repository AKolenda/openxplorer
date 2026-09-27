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
use ox_core::settings::Settings;
use tempfile::TempDir;

use crate::shared::AppContext;
use crate::theme::{Skin, ThemePreference};
use crate::window::BrowserWindow;

/// How long a test waits for the window to settle before it fails.
const PATIENCE: Duration = Duration::from_secs(8);

/// The standard fixture's visible items, as the details view sorts them:
/// folders first, then names in natural order.
pub(crate) const STANDARD_NAMES: [&str; 4] = ["Documents", "Notes 2.txt", "Notes 10.txt", "Résumé.txt"];

/// The application and skin of the test process.
struct Shared {
    app: gtk::Application,
    skin: Rc<Skin>,
}

impl Shared {
    fn new() -> Self {
        let app = gtk::Application::builder()
            .application_id("io.winspace.Development.Native.Test")
            .flags(gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&gio::Cancellable>)
            .expect("the private session bus accepts the test application");
        crate::window::install_accelerators(&app);
        let display = gdk::Display::default()
            .expect("window tests run on a private display: use native/tools/check.py");
        let skin = Rc::new(Skin::install(&display));
        Self { app, skin }
    }
}

thread_local! {
    /// Created on first use by GTK's test thread, which runs every window test.
    static SHARED: Shared = Shared::new();
}

/// The test application.
pub(crate) fn application() -> gtk::Application {
    SHARED.with(|shared| shared.app.clone())
}

/// The skin every test window shares.
pub(crate) fn skin() -> Rc<Skin> {
    SHARED.with(|shared| Rc::clone(&shared.skin))
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
/// When it does not hold within [`PATIENCE`]; the message names `what`.
pub(crate) fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    loop {
        settle();
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Runs the main loop for `duration`, for tests that prove something does
/// not happen.
pub(crate) fn wait_for(duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        settle();
        thread::sleep(Duration::from_millis(5));
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
pub(crate) struct Fixture {
    _directory: TempDir,
    root: PathBuf,
}

impl Fixture {
    fn empty() -> Self {
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
    pub fn standard() -> Self {
        let fixture = Self::empty();
        fs::create_dir(fixture.path("Documents")).expect("fixture subfolder");
        for name in ["Notes 10.txt", "Notes 2.txt", "Résumé.txt", ".private"] {
            fixture.write(name);
        }
        fixture
    }

    /// A folder of `count` files, enough to scroll.
    pub fn with_files(count: usize) -> Self {
        let fixture = Self::empty();
        for number in 0..count {
            fixture.write(&format!("file {number:04}.txt"));
        }
        fixture
    }

    /// Creates a small file called `name` in the folder.
    pub fn write(&self, name: &str) {
        fs::write(self.path(name), b"Synthetic test data\n").expect("fixture file");
    }

    /// The path of `name` in the folder.
    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// The folder itself.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder's canonical URI, as tabs store it.
    pub fn uri(&self) -> String {
        file_uri(&self.root)
    }

    /// The canonical URI of `name` in the folder.
    pub fn uri_of(&self, name: &str) -> String {
        file_uri(&self.path(name))
    }
}

/// A window of the test application, closed when dropped.
pub(crate) struct TestWindow {
    pub window: BrowserWindow,
    pub context: AppContext,
    /// Kept alive while the window reads and writes settings there.
    settings_directory: Rc<TempDir>,
}

impl TestWindow {
    /// A window with its own settings file, showing `uri` once listed.
    /// Opening files is recorded instead of starting applications.
    pub fn open(uri: &str) -> Self {
        let test = Self::without_tabs();
        test.show(uri);
        test
    }

    /// A window with its own settings file and no tab yet, for tests that
    /// watch the first listing.
    pub fn without_tabs() -> Self {
        let settings = tempfile::tempdir().expect("the test home has room for settings");
        let context = AppContext::new(skin(), Settings::open(settings.path()));
        context.record_launches();
        Self {
            window: BrowserWindow::new(&application(), &context),
            context,
            settings_directory: Rc::new(settings),
        }
    }

    /// A second window sharing this window's application state.
    pub fn open_beside(&self, uri: &str) -> Self {
        let beside = Self {
            window: BrowserWindow::new(&application(), &self.context),
            context: self.context.clone(),
            settings_directory: Rc::clone(&self.settings_directory),
        };
        beside.show(uri);
        beside
    }

    /// Adds a tab for `uri`, presents the window and waits for the listing.
    pub fn show(&self, uri: &str) {
        self.window
            .add_tab(uri)
            .expect("test locations are valid addresses");
        self.window.present();
        wait_until("the first listing", || {
            self.window.is_mapped() && !self.window.is_loading()
        });
    }

    /// The names shown, in display order.
    pub fn names(&self) -> Vec<String> {
        let model = self.window.folder_model();
        (0..model.n_items())
            .map(|position| model.name_at(position))
            .collect()
    }

    /// The names of the selected items, in display order.
    pub fn selected_names(&self) -> Vec<String> {
        let items = self.window.folder_model().selected_items();
        items.iter().map(|item| item.entry().name.clone()).collect()
    }

    /// Runs the window action `name` (`win.` omitted) with a string target.
    pub fn activate(&self, name: &str, target: Option<&str>) {
        let target = target.map(ToVariant::to_variant);
        WidgetExt::activate_action(&self.window, &format!("win.{name}"), target.as_ref())
            .expect("the window has the action");
    }

    /// The string state of the window action `name`.
    pub fn action_state(&self, name: &str) -> Option<String> {
        let state = self.window.lookup_action(name)?.state()?;
        state.get::<String>()
    }

    /// Waits until the active tab has finished listing.
    pub fn wait_for_listing(&self, what: &str) {
        wait_until(what, || !self.window.is_loading());
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        self.window.close();
        settle();
    }
}

/// Keeps the shared skin's theme choice for the length of a test that
/// changes it.
pub(crate) struct ThemeGuard(ThemePreference);

impl ThemeGuard {
    pub fn keep() -> Self {
        Self(skin().preference())
    }
}

impl Drop for ThemeGuard {
    fn drop(&mut self) {
        skin().set_preference(self.0);
    }
}

/// Saves a PNG of `window` into `$OX_NATIVE_CAPTURE_DIR`, when set, for
/// visual review of the layout.
pub(crate) fn capture(window: &BrowserWindow, filename: &str) {
    let Some(directory) = std::env::var_os("OX_NATIVE_CAPTURE_DIR") else {
        return;
    };
    let directory = PathBuf::from(directory);
    fs::create_dir_all(&directory).expect("capture directory");
    wait_for_frames(window, 3);
    let paintable = gtk::WidgetPaintable::new(Some(window));
    let snapshot = gtk::Snapshot::new();
    let width = f64::from(window.width());
    let height = f64::from(window.height());
    paintable.snapshot(&snapshot, width, height);
    let node = snapshot.to_node().expect("a mapped window has a render node");
    let renderer = window.renderer().expect("a mapped window has a renderer");
    let texture = renderer.render_texture(&node, None);
    texture
        .save_to_png(directory.join(filename))
        .expect("the capture directory is writable");
}

/// Waits until `window` has drawn `count` frames, so a capture shows the
/// finished layout.
fn wait_for_frames(window: &BrowserWindow, count: u32) {
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
